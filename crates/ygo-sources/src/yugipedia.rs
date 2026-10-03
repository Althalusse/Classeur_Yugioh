// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Yugipedia — les Set lists, seule description fidèle du contenu d'un set.
//!
//! Portage de la partie Yugipedia de `module/donnees/sync_reference.py`.
//!
//! # Pourquoi ce module existe
//!
//! Ni l'une ni l'autre des deux API ne décrit correctement la structure d'un
//! set. YGOPRODeck n'a **aucun lien tirage → illustration** : elle expose une
//! liste d'artworks et une liste de tirages sans jamais dire lequel va avec
//! lequel. YGOJSON a ce lien, mais tantôt il **énumère** (`LDK2-ENK01` reçoit
//! les huit artworks connus de Blue-Eyes alors que deux existent), tantôt il
//! est **aveugle** (`RA02` : sept cartes en artwork alternatif, aucune vue).
//!
//! La page « Set Card Lists » de Yugipedia, elle, décrit le contenu physique —
//! une ligne par groupe de tirages, avec ses raretés :
//!
//! ```text
//! LOCR-JP001; Blue-Eyes White Dragon…; Ultra Rare, Secret Rare
//! LOCR-JP001; Blue-Eyes White Dragon…; Ultra Rare, Prismatic Secret Rare,
//!             Grand Master Rare // description::(extended art)
//! ```
//!
//! Deux tirages normaux et trois extended art : cinq. C'est ce que contient la
//! boîte, et c'est ce que `cardinfo.db` doit contenir.
//!
//! # Module interdit de paraphrase
//!
//! Le cahier des charges classe ce parsing parmi les deux modules à porter
//! **à la lettre**. Le wikitext est une donnée que l'application ne contrôle
//! pas, écrite à la main par des contributeurs ; chaque tolérance du parser
//! Python correspond à une forme réellement rencontrée. En reformuler la
//! logique, c'est perdre celles qu'on n'a pas su reconnaître.

pub mod artwork;
pub mod structure;

use serde::Deserialize;

use crate::error::{Result, SourceError};
use crate::http::ClientHttp;

/// Point d'entrée de l'API MediaWiki de Yugipedia.
pub const API: &str = "https://yugipedia.com/api.php";

/// Nombre maximal de pages rendues par `prefixsearch` (`pslimit` du Python).
pub const LIMITE_RECHERCHE: u32 = 20;

// ─────────────────────────────────────────────────────────────────────────────
// Entrées de Set list
// ─────────────────────────────────────────────────────────────────────────────

/// Une ligne de `{{Set list}}`, telle que `parse_set_list` la rend.
///
/// Une entrée décrit **un groupe de tirages** : un numéro, un nom, et la liste
/// des raretés sous lesquelles ce groupe existe. Un même numéro peut donc
/// apparaître sur plusieurs entrées — typiquement une pour le cadre normal et
/// une pour l'extended art.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, Deserialize)]
pub struct EntreeSetList {
    /// Numéro de collection, `LOCR-JP001`.
    pub numero: String,
    /// Nom de la carte, liens wiki déroulés et entités HTML décodées.
    pub nom: String,
    /// Raretés de ce groupe de tirages.
    pub raretes: Vec<String>,
    /// La note contient-elle « extended art » ?
    ///
    /// Signal **partiel** : `ygo-app::overframe` en croise d'autres, la
    /// mention étant souvent absente du wikitext. Il est conservé tel quel
    /// pour rester comparable au Python.
    pub extended_art: bool,
    /// Ce qui suit `//` sur la ligne, nettoyé de ses espaces.
    pub note: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Parsing du wikitext
// ─────────────────────────────────────────────────────────────────────────────

const BALISE: &str = "{{Set list";

/// Corps de chaque bloc `{{Set list …}}` du wikitext.
///
/// Portage de `_extract_set_list_blocks`. Le découpage se fait au **compteur
/// d'accolades**, pas à la première fermeture rencontrée : les blocs
/// contiennent des modèles imbriqués (`{{Card list}}`, `{{Ruby}}`…) et une
/// recherche naïve de `}}` couperait au milieu.
///
/// # Écart assumé
///
/// Le Python cherche la balise dans une **copie minusculée** du wikitext tout
/// en découpant l'original — ce qui décale les indices si la mise en
/// minuscules change la longueur du texte (`İ` turc, par exemple). Ici la
/// recherche est insensible à la casse **ASCII** et travaille sur la chaîne
/// d'origine : même résultat sur tout wikitext réel, sans le décalage
/// possible.
pub fn extraire_blocs(wikitext: &str) -> Vec<&str> {
    let octets = wikitext.as_bytes();
    let balise = BALISE.as_bytes();
    let mut blocs = Vec::new();
    let mut depart = 0usize;

    while let Some(debut) = trouver_ascii_insensible(octets, balise, depart) {
        let mut profondeur = 0i32;
        let mut j = debut;
        // Le Python s'arrête à `len - 1` : une accolade isolée en toute fin de
        // texte ne peut pas ouvrir de paire.
        while j + 1 < octets.len() {
            match (octets.get(j), octets.get(j + 1)) {
                (Some(b'{'), Some(b'{')) => {
                    profondeur += 1;
                    j += 2;
                }
                (Some(b'}'), Some(b'}')) => {
                    profondeur -= 1;
                    j += 2;
                    if profondeur == 0 {
                        break;
                    }
                }
                _ => j += 1,
            }
        }
        let ouverture = debut + balise.len();
        let fermeture = j.saturating_sub(2);
        // Un découpage qui se croise donne un bloc vide, comme la tranche
        // Python `wikitext[a:b]` avec a > b — cas d'une balise ouverte que
        // rien ne referme, en fin de page tronquée.
        let bloc = if fermeture > ouverture
            && wikitext.is_char_boundary(ouverture)
            && wikitext.is_char_boundary(fermeture)
        {
            wikitext.get(ouverture..fermeture).unwrap_or("")
        } else {
            ""
        };
        blocs.push(bloc);
        // Le Python reprend exactement à `j`. La borne basse est une sécurité
        // sans effet observable : `j` dépasse toujours `debut` puisque la
        // balise fait dix caractères, et sans elle une balise en toute fin de
        // texte boucterait indéfiniment.
        depart = j.max(debut + 1);
    }
    blocs
}

/// Recherche d'un motif ASCII, insensible à la casse, à partir de `depuis`.
fn trouver_ascii_insensible(foin: &[u8], aiguille: &[u8], depuis: usize) -> Option<usize> {
    if aiguille.is_empty() || foin.len() < aiguille.len() {
        return None;
    }
    (depuis..=foin.len() - aiguille.len()).find(|&i| {
        foin.get(i..i + aiguille.len())
            .is_some_and(|f| f.eq_ignore_ascii_case(aiguille))
    })
}

/// Déroule les liens wiki et décode les entités HTML.
///
/// Portage de `_clean` : `[[Blue-Eyes White Dragon|Blue-Eyes]]` devient
/// `Blue-Eyes`, `[[Dark Magician]]` devient `Dark Magician`, puis les entités
/// HTML sont décodées et les espaces de bord retirés.
pub fn nettoyer(texte: &str) -> String {
    let sans_liens = regex_lien().replace_all(texte, "$1");
    html_escape::decode_html_entities(&sans_liens)
        .trim()
        .to_owned()
}

/// `\[\[(?:[^\]|]*\|)?([^\]]+)\]\]` — compilée une fois.
fn regex_lien() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        #[allow(clippy::expect_used)]
        regex::Regex::new(r"\[\[(?:[^\]|]*\|)?([^\]]+)\]\]").expect("motif de lien wiki constant")
    })
}

/// Analyse tous les blocs `{{Set list}}` d'une page.
///
/// Portage de `parse_set_list`. Le format d'une ligne de carte :
///
/// ```text
/// | LOCR-JP001; Blue-Eyes; Ultra Rare, Secret Rare // description::(extended art)
///   └ numéro    └ nom      └ raretés                 └ note
/// ```
///
/// Une ligne **sans point-virgule** n'est pas une carte mais une ligne de
/// paramètres du modèle (`|rarities=Common|region=JP`). Ses affectations
/// deviennent les valeurs par défaut des lignes suivantes du même bloc : une
/// carte sans colonne de raretés hérite alors de `rarities=`. C'est ce qui
/// rend les Set lists lisibles, et ce qui rend leur parsing sensible à
/// l'ordre.
pub fn parse_set_list(wikitext: &str) -> Vec<EntreeSetList> {
    let mut cartes = Vec::new();
    for corps in extraire_blocs(wikitext) {
        // Les paramètres du modèle, accumulés au fil du bloc.
        let mut defauts: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
        for ligne in corps.split('\n') {
            let ligne = ligne.trim();
            if ligne.is_empty() {
                continue;
            }
            if !ligne.contains(';') {
                for jeton in ligne.split('|') {
                    if let Some((cle, valeur)) = jeton.split_once('=') {
                        defauts.insert(cle.trim(), valeur.trim());
                    }
                }
                continue;
            }
            let entree = ligne.trim_start_matches('|').trim();
            let (principal, note) = match entree.split_once("//") {
                Some((avant, apres)) => (avant, apres.trim()),
                None => (entree, ""),
            };
            let champs: Vec<&str> = principal.split(';').map(str::trim).collect();

            let numero = champs.first().copied().unwrap_or_default();
            let nom = champs.get(1).map(|c| nettoyer(c)).unwrap_or_default();
            // Colonne de raretés vide ou absente : on hérite de l'en-tête.
            let raretes_brutes = match champs.get(2) {
                Some(c) if !c.is_empty() => *c,
                _ => defauts.get("rarities").copied().unwrap_or_default(),
            };

            cartes.push(EntreeSetList {
                numero: numero.to_owned(),
                nom,
                raretes: raretes_brutes
                    .split(',')
                    .map(str::trim)
                    .filter(|r| !r.is_empty())
                    .map(str::to_owned)
                    .collect(),
                extended_art: note.to_lowercase().contains("extended art"),
                note: note.to_owned(),
            });
        }
    }
    cartes
}

// ─────────────────────────────────────────────────────────────────────────────
// Réseau
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ReponseRecherche {
    query: RequeteRecherche,
}

#[derive(Deserialize)]
struct RequeteRecherche {
    #[serde(default)]
    prefixsearch: Vec<PageTrouvee>,
}

#[derive(Deserialize)]
struct PageTrouvee {
    title: String,
}

#[derive(Deserialize)]
struct ReponseParse {
    parse: ContenuParse,
}

#[derive(Deserialize)]
struct ContenuParse {
    wikitext: String,
    #[serde(default)]
    revid: Option<i64>,
}

/// Titres des pages « Set Card Lists » d'un set.
///
/// Portage de `resolve_set_pages`. La recherche se fait par **`prefixsearch`**
/// et non par recherche plein texte : celle-ci ne couvre pas ce namespace de
/// façon fiable — constat du Python, conservé.
///
/// `region` (`OCG-JP`, `TCG-EN`…) filtre sur le suffixe entre parenthèses du
/// titre. Sans région, tous les titres sont rendus dans l'ordre de l'API.
pub async fn resoudre_pages(
    client: &ClientHttp,
    nom_set: &str,
    region: Option<&str>,
) -> Result<Vec<String>> {
    let url = url_api(&[
        ("action", "query"),
        ("list", "prefixsearch"),
        ("pssearch", &format!("Set Card Lists:{nom_set}")),
        ("pslimit", &LIMITE_RECHERCHE.to_string()),
    ]);
    let reponse: ReponseRecherche = client.get_json(&url).await?;
    let mut titres: Vec<String> = reponse
        .query
        .prefixsearch
        .into_iter()
        .map(|p| p.title)
        .collect();
    if let Some(region) = region {
        let marqueur = format!("({region})");
        titres.retain(|t| t.contains(&marqueur));
    }
    Ok(titres)
}

/// Entrées de Set list d'une page, avec le numéro de révision.
///
/// Portage de `fetch_set_cards`. Les redirections sont suivies (`redirects`),
/// et le `revid` rendu sert de **garde d'idempotence** : un set dont la
/// révision n'a pas bougé n'est pas retraité.
pub async fn lire_set_list(
    client: &ClientHttp,
    titre: &str,
) -> Result<(Vec<EntreeSetList>, Option<i64>)> {
    let url = url_api(&[
        ("action", "parse"),
        ("page", titre),
        ("prop", "wikitext|revid"),
        ("redirects", "true"),
    ]);
    lire_parse(client, &url).await
}

/// Entrées de Set list d'une **révision figée**.
///
/// Absent du Python, et c'est précisément le point : le wikitext d'une page
/// vivante change, celui d'une révision jamais. C'est ce qui rend l'oracle de
/// cette passe reproductible — `overframe_sync` conserve le `revid` qu'a vu
/// chaque construction de base, et cette fonction permet d'y revenir.
pub async fn lire_set_list_revision(
    client: &ClientHttp,
    revid: i64,
) -> Result<(Vec<EntreeSetList>, Option<i64>)> {
    let url = url_api(&[
        ("action", "parse"),
        ("oldid", &revid.to_string()),
        ("prop", "wikitext|revid"),
    ]);
    lire_parse(client, &url).await
}

async fn lire_parse(client: &ClientHttp, url: &str) -> Result<(Vec<EntreeSetList>, Option<i64>)> {
    let reponse: ReponseParse = client.get_json(url).await?;
    let entrees = parse_set_list(&reponse.parse.wikitext);
    Ok((entrees, reponse.parse.revid))
}

/// Construit l'URL de l'API avec les paramètres communs du Python
/// (`format=json`, `formatversion=2`) et l'encodage de pourcentage.
pub(crate) fn url_api(parametres: &[(&str, &str)]) -> String {
    let mut url = format!("{API}?format=json&formatversion=2");
    for (cle, valeur) in parametres {
        url.push('&');
        url.push_str(cle);
        url.push('=');
        url.push_str(&encoder(valeur));
    }
    url
}

/// Encodage de pourcentage, jeu de caractères non réservés de la RFC 3986.
fn encoder(valeur: &str) -> String {
    let mut sortie = String::with_capacity(valeur.len());
    for octet in valeur.bytes() {
        match octet {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                sortie.push(octet as char);
            }
            _ => sortie.push_str(&format!("%{octet:02X}")),
        }
    }
    sortie
}

/// Une erreur d'API Yugipedia, si la réponse en porte une.
///
/// L'API MediaWiki répond `200 OK` avec un objet `error` plutôt qu'un statut
/// d'erreur : sans cette vérification, une page introuvable se lirait comme
/// une Set list vide.
pub fn erreur_api(valeur: &serde_json::Value) -> Option<SourceError> {
    let erreur = valeur.get("error")?;
    let info = erreur
        .get("info")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("erreur inconnue");
    Some(SourceError::Archive(format!("API Yugipedia : {info}")))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    /// Forme réelle d'une Set list, reprise de la page LOCR-JP.
    const WIKITEXT: &str = r#"
== Japanese ==
{{Set list|rarities=Common|region=JP|print=cardart|
LOCR-JP001; Blue-Eyes White Dragon, the White Phantom Beast; Ultra Rare, Secret Rare
LOCR-JP001; Blue-Eyes White Dragon, the White Phantom Beast; Ultra Rare, Prismatic Secret Rare, Grand Master Rare // description::(extended art)
LOCR-JP002; [[Deep-Eyes White Dragon|Deep-Eyes]]; Super Rare
LOCR-JP003; Pot of Greed
}}
"#;

    #[test]
    fn une_ligne_complete_se_decompose_en_quatre_champs() {
        let e = parse_set_list(WIKITEXT);
        assert_eq!(e.len(), 4);
        assert_eq!(e[0].numero, "LOCR-JP001");
        assert_eq!(e[0].nom, "Blue-Eyes White Dragon, the White Phantom Beast");
        assert_eq!(e[0].raretes, ["Ultra Rare", "Secret Rare"]);
        assert!(!e[0].extended_art);
        assert_eq!(e[0].note, "");
    }

    #[test]
    fn la_note_apres_double_barre_marque_l_extended_art() {
        let e = parse_set_list(WIKITEXT);
        assert!(e[1].extended_art);
        assert_eq!(e[1].note, "description::(extended art)");
        assert_eq!(
            e[1].raretes,
            ["Ultra Rare", "Prismatic Secret Rare", "Grand Master Rare"]
        );
    }

    #[test]
    fn les_liens_wiki_sont_deroules() {
        let e = parse_set_list(WIKITEXT);
        assert_eq!(e[2].nom, "Deep-Eyes");
        assert_eq!(nettoyer("[[Dark Magician]]"), "Dark Magician");
        assert_eq!(nettoyer("[[A|B]] et [[C]]"), "B et C");
        assert_eq!(nettoyer("  &amp; espaces  "), "& espaces");
    }

    #[test]
    fn une_ligne_sans_raretes_herite_de_l_en_tete() {
        // `rarities=Common` en tête de bloc ; LOCR-JP003 n'a pas de 3ᵉ champ.
        let e = parse_set_list(WIKITEXT);
        assert_eq!(e[3].numero, "LOCR-JP003");
        assert_eq!(e[3].raretes, ["Common"]);
    }

    #[test]
    fn un_champ_de_raretes_vide_herite_aussi() {
        let w = "{{Set list|rarities=Rare|\nSET-EN001; Carte;\n}}";
        assert_eq!(parse_set_list(w)[0].raretes, ["Rare"]);
    }

    #[test]
    fn sans_en_tete_ni_colonne_les_raretes_sont_vides() {
        let w = "{{Set list|\nSET-EN001; Carte;\n}}";
        assert!(parse_set_list(w)[0].raretes.is_empty());
    }

    #[test]
    fn les_modeles_imbriques_ne_coupent_pas_le_bloc() {
        // Une fermeture naïve au premier `}}` s'arrêterait dans {{Ruby}}.
        let w = "{{Set list|rarities=Common|\n\
                 SET-JP001; {{Ruby|青眼|ブルーアイズ}}; Ultra Rare\n\
                 SET-JP002; Deuxième; Rare\n}}";
        let e = parse_set_list(w);
        assert_eq!(e.len(), 2);
        assert_eq!(e[1].numero, "SET-JP002");
        // Le nettoyage ne déroule que les liens `[[…]]` : le modèle reste tel
        // quel dans le nom, comme en Python.
        assert_eq!(e[0].nom, "{{Ruby|青眼|ブルーアイズ}}");
    }

    #[test]
    fn un_modele_imbrique_ne_deborde_pas_sur_le_bloc_suivant() {
        // Le cas qui discrimine vraiment le compteur d'accolades. Sans lui, la
        // profondeur ne revient jamais à zéro : le premier bloc avale le reste
        // de la page, et les entrées du second sont comptées deux fois.
        let w = "{{Set list|rarities=Common|\n\
                 A-EN001; {{Ruby|x|y}}; Rare\n}}\n\
                 texte\n\
                 {{Set list|rarities=Common|\n\
                 B-EN001; Deux; Rare\n}}";
        assert_eq!(
            extraire_blocs(w),
            [
                "|rarities=Common|\nA-EN001; {{Ruby|x|y}}; Rare\n",
                "|rarities=Common|\nB-EN001; Deux; Rare\n"
            ]
        );
        let e = parse_set_list(w);
        assert_eq!(e.len(), 2, "deux entrées, pas trois");
        assert_eq!(e[0].numero, "A-EN001");
        assert_eq!(e[1].numero, "B-EN001");
    }

    #[test]
    fn les_champs_sont_detoures_un_a_un() {
        // `[c.strip() for c in main.split(";")]` : chaque champ, pas seulement
        // la ligne. Sans ça le numéro garde son espace de fin et ne
        // correspond plus à aucun `set_code`.
        let w = "{{Set list|rarities=Common|\n\
                 A-EN001 ; Nom de carte ; Ultra Rare , Secret Rare \n}}";
        let e = parse_set_list(w);
        assert_eq!(e[0].numero, "A-EN001");
        assert_eq!(e[0].nom, "Nom de carte");
        assert_eq!(e[0].raretes, ["Ultra Rare", "Secret Rare"]);
    }

    #[test]
    fn plusieurs_blocs_sont_concatenes() {
        let w = "{{Set list|rarities=Common|\nA-EN001; Un; Rare\n}}\n\
                 texte intercalaire\n\
                 {{Set list|rarities=Common|\nA-FR001; Un; Rare\n}}";
        let e = parse_set_list(w);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].numero, "A-EN001");
        assert_eq!(e[1].numero, "A-FR001");
    }

    #[test]
    fn la_balise_est_reconnue_quelle_que_soit_la_casse() {
        let w = "{{set LIST|rarities=Common|\nA-EN001; Un; Rare\n}}";
        assert_eq!(parse_set_list(w).len(), 1);
    }

    #[test]
    fn le_wikitext_sans_bloc_ne_donne_rien() {
        assert!(parse_set_list("== Rien ici ==\ndu texte").is_empty());
        assert!(parse_set_list("").is_empty());
        assert!(extraire_blocs("== Rien ici ==").is_empty());
        // Balise ouverte que rien ne referme : le Python rend UN bloc vide,
        // pas zéro bloc. Vérifié contre lui.
        assert_eq!(extraire_blocs("{{Set list"), [""]);
        assert!(parse_set_list("{{Set list").is_empty());
    }

    #[test]
    fn les_lignes_de_parametres_ne_sont_pas_des_cartes() {
        // Sans point-virgule → paramètres, pas une carte.
        let w = "{{Set list|\n|rarities=Common|region=JP\nA-EN001; Un; Rare\n}}";
        let e = parse_set_list(w);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].numero, "A-EN001");
    }

    #[test]
    fn le_wikitext_japonais_ne_decale_pas_le_decoupage() {
        let w = "{{Set list|rarities=Common|\n\
                 LOCR-JP001; 青眼の白龍; Ultra Rare\n}}";
        let e = parse_set_list(w);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].nom, "青眼の白龍");
    }

    #[test]
    fn l_url_encode_les_espaces_et_les_deux_points() {
        let url = url_api(&[("pssearch", "Set Card Lists:Limit Over")]);
        assert!(url.starts_with("https://yugipedia.com/api.php?format=json&formatversion=2"));
        assert!(url.contains("pssearch=Set%20Card%20Lists%3ALimit%20Over"));
    }

    #[test]
    fn une_erreur_d_api_arrive_avec_un_statut_200() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"error":{"code":"missingtitle","info":"La page n'existe pas"}}"#,
        )
        .unwrap();
        let e = erreur_api(&v).unwrap();
        assert!(e.to_string().contains("La page n'existe pas"));
        assert!(erreur_api(&serde_json::json!({"parse": {}})).is_none());
    }
}
