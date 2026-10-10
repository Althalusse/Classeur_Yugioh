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

use std::collections::{HashMap, HashSet};

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
        .chars()
        .filter(|c| !invisible(*c))
        .collect::<String>()
        .trim()
        .to_owned()
}

/// Un caractère de mise en forme **invisible** : marques de direction,
/// espaces de largeur nulle, BOM.
///
/// # Pourquoi — 2026-10-10
///
/// La simulation sur 50 sets en a trouvé dans les Set lists :
/// « `Sea Archiver\u{200E}\u{200E}` » (`SD33-JP003`), « `Speedroid
/// Hexasaucer\u{200E}` » (`19PP-JP008`)… Invisibles à l'écran, ils font
/// échouer toute comparaison de noms — c'est sans doute ainsi que YGOJSON a
/// perdu ces tirages. Les retirer ici sert toute la chaîne : numéros absents,
/// passe artworks, variantes.
fn invisible(c: char) -> bool {
    matches!(
        c,
        '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}' | '\u{FEFF}'
    )
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

/// Combien de titres une requête peut regrouper — la limite de MediaWiki pour
/// un client ordinaire.
pub const TITRES_PAR_REQUETE: usize = 50;

/// Réponse de `prop=imageinfo` (`formatversion=2`).
#[derive(Debug, Default, Deserialize)]
struct ReponseFichiers {
    #[serde(default)]
    query: Option<QueryFichiers>,
}

#[derive(Debug, Default, Deserialize)]
struct QueryFichiers {
    #[serde(default)]
    normalized: Vec<Normalisation>,
    #[serde(default)]
    pages: Vec<PageFichier>,
}

#[derive(Debug, Deserialize)]
struct Normalisation {
    from: String,
    to: String,
}

#[derive(Debug, Deserialize)]
struct PageFichier {
    title: String,
    #[serde(default)]
    missing: bool,
    #[serde(default)]
    invalid: bool,
    #[serde(default)]
    imageinfo: Vec<serde_json::Value>,
}

/// La réponse rend-elle une page — présente ou `missing` — par fichier
/// demandé ?
fn complete(reponse: &ReponseFichiers, demandes: usize) -> bool {
    reponse
        .query
        .as_ref()
        .is_some_and(|q| q.pages.len() >= demandes)
}

/// Les noms de la demande que la réponse dit présents — fonction **pure**.
///
/// MediaWiki **normalise** les titres (première lettre en capitale, `_` en
/// espace) et le dit dans `normalized` : la correspondance repasse par là,
/// sans quoi un nom présent passerait pour absent.
fn presents(reponse: &ReponseFichiers, demandes: &[String]) -> HashSet<String> {
    let Some(q) = &reponse.query else {
        return HashSet::new();
    };
    let normalise: HashMap<&str, &str> = q
        .normalized
        .iter()
        .map(|n| (n.from.as_str(), n.to.as_str()))
        .collect();
    let existe: HashSet<&str> = q
        .pages
        .iter()
        .filter(|p| !p.missing && !p.invalid && !p.imageinfo.is_empty())
        .map(|p| p.title.as_str())
        .collect();
    demandes
        .iter()
        .filter(|nom| {
            let titre = format!("File:{nom}");
            let final_ = normalise.get(titre.as_str()).copied().unwrap_or(&titre);
            existe.contains(final_)
        })
        .cloned()
        .collect()
}

/// Parmi ces noms de fichiers, lesquels existent sur Yugipedia.
///
/// # Pourquoi — 2026-10-02
///
/// `Yugipedia:API` : *« The content of multiple pages can be bundled together
/// into a single request with a pipe »*. Vérifier 27 images de repli coûte
/// ainsi **une** requête au lieu de 27 téléchargements voués au 404 — et
/// seules celles qui existent sont ensuite retéléchargées.
///
/// Ces réponses ne passent **pas** par le cache de 30 jours
/// ([`crate::cache::duree_de_cache`]) : elles sont faites pour voir ce qui a
/// changé, une fois par mise à jour de la base.
///
/// # Errors
///
/// Rend une erreur sur une panne réseau ou une réponse illisible.
pub async fn fichiers_existants(client: &ClientHttp, noms: &[String]) -> Result<HashSet<String>> {
    let mut trouves = HashSet::new();
    for lot in noms.chunks(TITRES_PAR_REQUETE) {
        let titres: Vec<String> = lot.iter().map(|n| format!("File:{n}")).collect();
        let url = url_api(&[
            ("action", "query"),
            ("prop", "imageinfo"),
            ("iiprop", "url"),
            ("titles", &titres.join("|")),
        ]);
        let valeur: serde_json::Value = client.get_json(&url).await?;
        if let Some(e) = erreur_api(&valeur) {
            return Err(e);
        }
        let reponse: ReponseFichiers = serde_json::from_value(valeur)
            .map_err(|e| SourceError::deserialisation(url.clone(), e))?;
        // Une réponse qui ne parle pas de chaque fichier demandé n'est pas un
        // « absent » : c'est une réponse qu'on ne comprend pas. La lire comme
        // « rien n'existe » serait faux sans que rien ne le dise.
        if !complete(&reponse, lot.len()) {
            return Err(SourceError::Statut {
                url: url.clone(),
                statut: 0,
            });
        }
        trouves.extend(presents(&reponse, lot));
    }
    Ok(trouves)
}

/// Réponse d'une lecture de fiches (`redirects` + `revisions`, `formatversion=2`).
#[derive(Debug, Default, Deserialize)]
struct ReponseTitres {
    #[serde(default)]
    query: Option<QueryTitres>,
}

#[derive(Debug, Default, Deserialize)]
struct QueryTitres {
    #[serde(default)]
    normalized: Vec<Normalisation>,
    #[serde(default)]
    redirects: Vec<Normalisation>,
    #[serde(default)]
    pages: Vec<PageTitre>,
}

#[derive(Debug, Deserialize)]
struct PageTitre {
    title: String,
    #[serde(default)]
    missing: bool,
    #[serde(default)]
    invalid: bool,
    #[serde(default)]
    revisions: Vec<RevisionFiche>,
}

/// Une révision de fiche.
///
/// Yugipedia tourne sur un MediaWiki qui **ignore** `rvslots` (« *Unrecognized
/// parameter* ») et rend le wikitext directement dans `content` — vu le
/// 2026-10-10 sur la fiche de `LOCH-JP013`. Les versions récentes le rangent
/// dans `slots.main.content`. Les deux formes sont lues : la première faisait
/// passer une fiche complète pour une fiche sans password.
#[derive(Debug, Deserialize)]
struct RevisionFiche {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    slots: Option<SlotsFiche>,
}

impl RevisionFiche {
    /// Le wikitext, où que la version de MediaWiki l'ait rangé.
    fn wikitext(&self) -> Option<&str> {
        self.content.as_deref().or_else(|| {
            self.slots
                .as_ref()
                .and_then(|s| s.main.as_ref())
                .map(|m| m.content.as_str())
        })
    }
}

#[derive(Debug, Deserialize)]
struct SlotsFiche {
    #[serde(default)]
    main: Option<ContenuFiche>,
}

#[derive(Debug, Deserialize)]
struct ContenuFiche {
    #[serde(default)]
    content: String,
}

/// Ce que Yugipedia dit d'une carte : sa fiche, et son password.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fiche {
    /// Titre de la fiche, redirections suivies.
    pub titre: String,
    /// Le password imprimé sur la carte — l'identifiant YGOPRODeck — s'il est
    /// renseigné. Absent pour une carte qui n'en porte pas.
    pub password: Option<i64>,
}

/// Le password d'une fiche de carte, lu dans son wikitext.
///
/// ```text
/// {{CardTable2
/// | password    = 75787708
/// ```
fn password_du_wikitext(wikitext: &str) -> Option<i64> {
    wikitext.lines().find_map(|ligne| {
        let (cle, valeur) = ligne
            .trim_start_matches(|c: char| c == '|' || c.is_whitespace())
            .split_once('=')?;
        if cle.trim() != "password" {
            return None;
        }
        let chiffres: String = valeur
            .trim()
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        chiffres.parse::<i64>().ok().filter(|p| *p > 0)
    })
}

/// La fiche de chaque nom demandé — fonction **pure**.
///
/// Le chemin est celui de MediaWiki : normalisation (capitale initiale, `_`),
/// puis redirection, éventuellement en chaîne. Un nom dont la page finale
/// n'existe pas n'est pas rendu : on ne devine pas.
fn fiches_de(reponse: &ReponseTitres, demandes: &[String]) -> HashMap<String, Fiche> {
    let Some(q) = &reponse.query else {
        return HashMap::new();
    };
    let normalise: HashMap<&str, &str> = q
        .normalized
        .iter()
        .map(|n| (n.from.as_str(), n.to.as_str()))
        .collect();
    let redirige: HashMap<&str, &str> = q
        .redirects
        .iter()
        .map(|r| (r.from.as_str(), r.to.as_str()))
        .collect();
    let pages: HashMap<&str, &PageTitre> = q
        .pages
        .iter()
        .filter(|p| !p.missing && !p.invalid)
        .map(|p| (p.title.as_str(), p))
        .collect();
    let mut fiches = HashMap::new();
    for nom in demandes {
        let mut titre = normalise.get(nom.as_str()).copied().unwrap_or(nom.as_str());
        // Une chaîne de redirections reste courte ; la borne évite une boucle
        // sur une réponse aberrante.
        for _ in 0..5 {
            match redirige.get(titre) {
                Some(suivant) => titre = suivant,
                None => break,
            }
        }
        if let Some(page) = pages.get(titre) {
            let password = page
                .revisions
                .first()
                .and_then(RevisionFiche::wikitext)
                .and_then(password_du_wikitext);
            fiches.insert(
                nom.clone(),
                Fiche {
                    titre: titre.to_owned(),
                    password,
                },
            );
        }
    }
    fiches
}

/// La fiche Yugipedia de chacun de ces noms de carte : titre et password.
///
/// # Pourquoi — 2026-10-10
///
/// Une Set Card List peut nommer une carte autrement que la base. `LOCH-JP013`
/// est écrite « *Odd-Eyes Pendulum Dragon, Four Heavenly Dragons* » — c'est le
/// titre actuel de sa fiche — quand YGOJSON, et donc `cardinfo.db`, la
/// connaissent sous « *Odd-Eyes Pendulum Dragon of the Four Heavenly
/// Dragons* ». Aucun nom ne fait le pont ; le **password** (75787708), si :
/// il est le même partout, et c'est l'identifiant YGOPRODeck de la base.
///
/// Une requête pour cinquante noms (`titles=A|B|…`, contenu des fiches
/// compris), comme le demande `Yugipedia:API`. Les réponses passent par le
/// cache de 30 jours : une fiche ne change pas de password.
///
/// # Errors
///
/// Rend une erreur sur une panne réseau, une réponse illisible ou incomplète.
pub async fn fiches(client: &ClientHttp, noms: &[String]) -> Result<HashMap<String, Fiche>> {
    let mut trouvees = HashMap::new();
    for lot in noms.chunks(TITRES_PAR_REQUETE) {
        let url = url_api(&[
            ("action", "query"),
            ("redirects", "true"),
            ("prop", "revisions"),
            ("rvprop", "content"),
            ("titles", &lot.join("|")),
        ]);
        let valeur: serde_json::Value = client.get_json(&url).await?;
        if let Some(e) = erreur_api(&valeur) {
            return Err(e);
        }
        let reponse: ReponseTitres = serde_json::from_value(valeur)
            .map_err(|e| SourceError::deserialisation(url.clone(), e))?;
        // Une page par titre demandé, présente ou `missing` ; aucune, c'est une
        // réponse qu'on ne comprend pas — pas une absence.
        let pages = reponse.query.as_ref().map_or(0, |q| q.pages.len());
        if pages == 0 {
            return Err(SourceError::Statut {
                url: url.clone(),
                statut: 0,
            });
        }
        trouvees.extend(fiches_de(&reponse, lot));
    }
    Ok(trouvees)
}

/// Le nom de fichier d'une adresse d'image Yugipedia, **décodé** :
/// `…/a/ab/Dark_Magician%27s-X.png` donne `Dark_Magician's-X.png`.
///
/// ```
/// use ygo_sources::yugipedia::nom_de_fichier;
/// assert_eq!(
///     nom_de_fichier("https://ms.yugipedia.com//9/92/TheFluteofGuidingDragon-LOCR-JP-UR.png").as_deref(),
///     Some("TheFluteofGuidingDragon-LOCR-JP-UR.png")
/// );
/// assert_eq!(nom_de_fichier("https://ms.yugipedia.com//a/ab/A%27s%20B.png").as_deref(), Some("A's B.png"));
/// assert_eq!(nom_de_fichier("https://ms.yugipedia.com//"), None);
/// ```
#[must_use]
pub fn nom_de_fichier(url: &str) -> Option<String> {
    let sans_requete = url.split(['?', '#']).next()?;
    // Le chemin seul : l'hôte n'est jamais un nom de fichier.
    let sans_schema = sans_requete
        .split_once("://")
        .map_or(sans_requete, |(_, r)| r);
    let chemin = sans_schema.split_once('/').map_or("", |(_, c)| c);
    let dernier = chemin.trim_end_matches('/').rsplit('/').next()?;
    if dernier.is_empty() || dernier.contains(':') {
        return None;
    }
    let octets = dernier.as_bytes();
    let mut sortie = Vec::with_capacity(octets.len());
    let mut i = 0;
    while i < octets.len() {
        let octet = octets.get(i).copied().unwrap_or_default();
        let hexa = octets
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (octet, hexa) {
            (b'%', Some(v)) => {
                sortie.push(v);
                i += 3;
            }
            _ => {
                sortie.push(octet);
                i += 1;
            }
        }
    }
    String::from_utf8(sortie).ok()
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
mod tests_fichiers {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    /// Réponse réelle de forme `formatversion=2` : un fichier présent, un
    /// absent, et une normalisation (`_` → espace).
    #[test]
    fn la_reponse_dit_lesquels_existent_normalisation_comprise() {
        let reponse: ReponseFichiers = serde_json::from_value(serde_json::json!({
            "batchcomplete": true,
            "query": {
                "normalized": [{"fromencoded": false, "from": "File:Dark_Magician-LOB-EN-UR-1E.png", "to": "File:Dark Magician-LOB-EN-UR-1E.png"}],
                "pages": [
                    {"ns": 6, "title": "File:Dark Magician-LOB-EN-UR-1E.png", "pageid": 1,
                     "imageinfo": [{"url": "https://ms.yugipedia.com//a/ab/Dark_Magician-LOB-EN-UR-1E.png"}]},
                    {"ns": 6, "title": "File:TheFluteofGuidingDragon-LOCR-JP-UR.png", "missing": true}
                ]
            }
        }))
        .unwrap();
        let demandes = vec![
            "Dark_Magician-LOB-EN-UR-1E.png".to_owned(),
            "TheFluteofGuidingDragon-LOCR-JP-UR.png".to_owned(),
        ];
        let p = presents(&reponse, &demandes);
        assert!(p.contains("Dark_Magician-LOB-EN-UR-1E.png"));
        assert!(!p.contains("TheFluteofGuidingDragon-LOCR-JP-UR.png"));
        assert!(presents(&ReponseFichiers::default(), &demandes).is_empty());
        assert!(complete(&reponse, 2));
    }

    /// Une réponse sans `query` — ou avec moins de pages que de fichiers
    /// demandés — n'est pas « tout est absent ».
    #[test]
    fn une_reponse_incomplete_n_est_pas_un_absent() {
        let vide: ReponseFichiers =
            serde_json::from_value(serde_json::json!({"batchcomplete": true})).unwrap();
        assert!(!complete(&vide, 1));
        let partielle: ReponseFichiers = serde_json::from_value(serde_json::json!({
            "query": {"pages": [{"title": "File:A.png", "missing": true}]}
        }))
        .unwrap();
        assert!(complete(&partielle, 1));
        assert!(!complete(&partielle, 2));
    }
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

    /// Normalisation, puis redirection, puis password lu dans la fiche ; une
    /// page absente ne rend rien.
    #[test]
    fn les_caracteres_invisibles_sont_retires_des_noms() {
        assert_eq!(nettoyer("Sea Archiver\u{200E}\u{200E}"), "Sea Archiver");
        assert_eq!(
            nettoyer("[[Speedroid Hexasaucer]]\u{200E}"),
            "Speedroid Hexasaucer"
        );
        assert_eq!(nettoyer("\u{FEFF}Dark\u{200B} Magician"), "Dark Magician");
        assert_eq!(
            nettoyer("Danger! Disturbance! Disorder!"),
            "Danger! Disturbance! Disorder!"
        );
    }

    #[test]
    fn chaque_nom_rend_sa_fiche_et_son_password() {
        let reponse: ReponseTitres = serde_json::from_value(serde_json::json!({
            "query": {
                "normalized": [{"from": "dark magician", "to": "Dark magician"}],
                "redirects": [{"from": "Dark magician", "to": "Dark Magician"}],
                "pages": [
                    {"pageid": 1164647, "title": "Odd-Eyes Pendulum Dragon, Four Heavenly Dragons",
                     "revisions": [{"slots": {"main": {"content":
                        "{{CardTable2\n| ja_name = 四天の龍\n| password       = 75787708\n| attribute = DARK\n}}"}}}]},
                    {"title": "Dark Magician",
                     "revisions": [{"contentformat": "text/x-wiki", "contentmodel": "wikitext",
                                    "content": "{{CardTable2\n| password              = 46986414\n}}"}]},
                    {"title": "Inexistante", "missing": true}
                ]
            }
        }))
        .unwrap();
        let demandes = [
            "Odd-Eyes Pendulum Dragon, Four Heavenly Dragons".to_owned(),
            "dark magician".to_owned(),
            "Inexistante".to_owned(),
        ];
        let f = fiches_de(&reponse, &demandes);
        let oe = f
            .get("Odd-Eyes Pendulum Dragon, Four Heavenly Dragons")
            .unwrap();
        assert_eq!(oe.password, Some(75_787_708));
        let dm = f.get("dark magician").unwrap();
        assert_eq!(
            (dm.titre.as_str(), dm.password),
            ("Dark Magician", Some(46_986_414))
        );
        assert!(!f.contains_key("Inexistante"));
    }

    #[test]
    fn un_password_vide_ou_absent_ne_rend_rien() {
        assert_eq!(password_du_wikitext("| password = \n| x = 1"), None);
        assert_eq!(password_du_wikitext("| passwords = 12"), None);
        assert_eq!(password_du_wikitext("rien"), None);
        assert_eq!(password_du_wikitext("|password=00012345"), Some(12_345));
    }

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
