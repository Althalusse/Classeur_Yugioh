// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! La Set list Yugipedia comme référence de **structure**.
//!
//! Portage de `donnees/structure_yugipedia.py` (272 l.).
//!
//! # Pourquoi cette source, et pas une API
//!
//! Aucune des deux API ne décrit correctement le contenu physique d'un set :
//!
//! | source | ce qui manque |
//! |---|---|
//! | YGOPRODeck | **aucun lien** tirage → illustration ; `card_images[0]` est un choix arbitraire de l'appelant |
//! | YGOJSON | le lien existe, mais selon les sets il **énumère** (`LDK2-ENK01` : les 8 artworks de Blue-Eyes sur un numéro qui en a 2) ou il est **aveugle** (`RA02` : 7 cartes en artwork alternatif, aucune vue) |
//!
//! La page « Set Card Lists », elle, décrit une ligne par groupe de tirages
//! avec ses raretés — exactement le contenu de la boîte.
//!
//! # Ce module ne réinvente rien
//!
//! Le parsing est déjà celui de [`crate::yugipedia`], qui sert `overframe`
//! depuis juin 2026. Ce module ne fait que **développer** les entrées en
//! *slots* : un slot par (numéro, rareté, variante).

use std::collections::BTreeMap;

use super::EntreeSetList;

/// Régions OCG — choisissent la page `(OCG-JP)` plutôt que `(TCG-EN)`.
const REGIONS_OCG: [&str; 7] = ["JP", "JA", "KR", "KO", "SC", "TC", "AE"];

/// Annotations qui ne désignent **pas** une variante d'illustration.
///
/// `alternate password` : même illustration, autre code Konami — relevé sur
/// `RA03`. Sans cette exception, tous ces tirages passeraient pour des
/// variantes.
const NON_VARIANTES: [&str; 1] = ["password"];

/// Un tirage de la Set list : une rareté d'un numéro, avec sa variante.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot {
    /// Numéro de collection, en majuscules.
    pub numero: String,
    /// Nom de la carte.
    pub nom: String,
    /// Libellé de rareté, tel que le wiki l'écrit.
    pub rarete: String,
    /// `""`, `"AA"` ou `"EA"`.
    pub variante: String,
}

/// Clé de comparaison d'un libellé de rareté.
///
/// Portage de `normaliser_rarete`. Les libellés du wiki ne sont **pas** fiables
/// au caractère près : « PLatinum Secret Rare » et « PLatium Secret Rare » ont
/// tous deux été relevés sur `RA05`. Tout ce qui n'est pas alphanumérique
/// minuscule disparaît.
pub fn normaliser_rarete(rarete: &str) -> String {
    rarete
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        .collect()
}

/// Le type de variante d'une entrée : `"EA"`, `"AA"` ou `""`.
///
/// Portage de `variante_depuis_entree`. Le parser pose déjà `extended_art`
/// quand la note contient « extended art » ; ce sont les **autres**
/// formulations qu'il faut rattraper ici, et les sets réels en ont beaucoup :
/// « stamp artwork » (`RA05`), « alternate art » / « alternate artwork »
/// (`RA02`, `RA03`), « new artwork » (`RA04`), « 3rd artwork »,
/// « (2nd, 5th and 6th artwork) »…
///
/// # Un repli volontairement large
///
/// À défaut de formulation connue, **toute** annotation contenant `art` vaut
/// `AA`. C'est délibéré : une annotation d'illustration non répertoriée vaut
/// mieux traitée comme variante qu'ignorée — sauf celles qui parlent d'autre
/// chose, comme « alternate password ».
pub fn variante_depuis_entree(entree: &EntreeSetList) -> &'static str {
    if entree.extended_art {
        return "EA";
    }
    let note = entree.note.to_lowercase();
    let Some(texte) = description(&note) else {
        return "";
    };
    let texte = texte.trim();
    if NON_VARIANTES.iter().any(|mot| texte.contains(mot)) {
        return "";
    }
    if texte.contains("extended art") {
        return "EA";
    }
    if texte.contains("art") {
        return "AA";
    }
    ""
}

/// Le contenu de `description::(…)` dans une note déjà en minuscules.
///
/// Portage de `_RE_DESCRIPTION`. Sans expression régulière : la parenthèse
/// fermante est la **première** rencontrée, comme `[^)]*` l'impose.
fn description(note_minuscule: &str) -> Option<&str> {
    let debut = note_minuscule.find("description::(")? + "description::(".len();
    let reste = note_minuscule.get(debut..)?;
    let fin = reste.find(')')?;
    reste.get(..fin)
}

/// Combien d'illustrations une ligne de Set list annonce pour **chacun** de
/// ses tirages : `(four unique artworks)` → 4 ; 1 sans annonce.
///
/// # Pourquoi — 2026-10-16
///
/// `LCKC-EN001` Blue-Eyes White Dragon est imprimé en **quatre**
/// illustrations sous un seul numéro et une seule rareté, `LCKC-EN046`
/// Crush Card Virus en **deux**. La Set list ne leur donne qu'une ligne,
/// annotée `(four unique artworks)` et `(two unique artworks)` ; la galerie
/// du set en montre quatre et deux. Lue comme une variante unique, cette
/// ligne ne demandait qu'un tirage : le classeur n'en avait qu'un.
///
/// # La règle
///
/// Seul un **cardinal** compte — en lettres (`two` à `twelve`) ou en
/// chiffres —, suivi au besoin d'`unique`, `different` ou `distinct`, puis
/// d'`artworks` **au pluriel**. Un ordinal ne compte pas : `(6th artwork)` et
/// `(2nd, 5th and 6th artwork)` désignent **une** illustration — la sixième
/// de la carte —, pas six.
///
/// ```
/// use ygo_sources::yugipedia::structure::nombre_d_illustrations;
/// use ygo_sources::yugipedia::EntreeSetList;
/// let entree = |note: &str| EntreeSetList {
///     numero: "LCKC-EN001".into(),
///     nom: "Blue-Eyes White Dragon".into(),
///     raretes: vec!["Ultra Rare".into()],
///     extended_art: false,
///     note: note.into(),
/// };
/// assert_eq!(nombre_d_illustrations(&entree("description::(four unique artworks)")), 4);
/// assert_eq!(nombre_d_illustrations(&entree("description::(6th artwork)")), 1);
/// ```
#[must_use]
pub fn nombre_d_illustrations(entree: &EntreeSetList) -> usize {
    const ADJECTIFS: [&str; 3] = ["unique", "different", "distinct"];
    let note = entree.note.to_lowercase();
    let Some(texte) = description(&note) else {
        return 1;
    };
    let mots: Vec<&str> = texte
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|m| !m.is_empty())
        .collect();
    for (i, mot) in mots.iter().enumerate() {
        if *mot != "artworks" {
            continue;
        }
        // On remonte les adjectifs jusqu'au nombre qui les précède.
        let mut j = i;
        while let Some(precedent) = j.checked_sub(1).and_then(|k| mots.get(k)) {
            if !ADJECTIFS.contains(precedent) {
                break;
            }
            j -= 1;
        }
        if let Some(n) = j
            .checked_sub(1)
            .and_then(|k| mots.get(k))
            .and_then(|m| cardinal(m))
        {
            return n;
        }
    }
    1
}

/// `four` → 4, `3` → 3 ; `None` pour ce qui n'est pas un cardinal de 2 à 26
/// — au-delà, ce n'est plus une galerie, c'est une coquille.
fn cardinal(mot: &str) -> Option<usize> {
    const EN_LETTRES: [&str; 11] = [
        "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve",
    ];
    let n = match EN_LETTRES.iter().position(|m| *m == mot) {
        Some(i) => i + 2,
        None if mot.bytes().all(|b| b.is_ascii_digit()) => mot.parse().ok()?,
        None => return None,
    };
    (2..=26).contains(&n).then_some(n)
}

/// Développe les entrées d'une Set list en slots.
///
/// Portage de la seconde moitié de `charger_structure` — celle qui ne touche
/// pas au réseau. Une entrée sans numéro est ignorée ; une rareté vide aussi.
///
/// Une entrée qui annonce **plusieurs illustrations** pour un même tirage
/// ([`nombre_d_illustrations`], `LCKC-EN001`) donne autant de slots par
/// rareté : chaque illustration est une carte à part, que le classeur doit
/// montrer et compter à part. Une entrée sans variante n'est jamais
/// démultipliée — sans annotation d'illustration, rien ne dit laquelle.
pub fn slots(entrees: &[EntreeSetList]) -> Vec<Slot> {
    let mut sortie = Vec::new();
    for entree in entrees {
        let numero = entree.numero.trim().to_uppercase();
        if numero.is_empty() {
            continue;
        }
        let variante = variante_depuis_entree(entree);
        let illustrations = if variante.is_empty() {
            1
        } else {
            nombre_d_illustrations(entree)
        };
        for rarete in &entree.raretes {
            let rarete = rarete.trim();
            if rarete.is_empty() {
                continue;
            }
            for _ in 0..illustrations {
                sortie.push(Slot {
                    numero: numero.clone(),
                    nom: entree.nom.clone(),
                    rarete: rarete.to_owned(),
                    variante: variante.to_owned(),
                });
            }
        }
    }
    sortie
}

/// `(numéro, rareté normalisée)` vers la liste de ses variantes.
pub type IndexSlots = BTreeMap<(String, String), Vec<String>>;

/// Indexe les slots pour comparaison avec le classeur.
///
/// Portage de `slots_par_numero_rarete`. Les variantes sont triées **normales
/// d'abord** — le même ordre que les lignes de classeur triées par
/// `(extended_art, rowid)`, ce qui est toute la raison d'être de ce tri :
/// l'appariement se fait ensuite par rang.
///
/// # Une clé de tri qui ne sert à rien, gardée quand même
///
/// La clé `(la variante est-elle vide ?, la variante)` donne **exactement** le
/// même ordre qu'un tri sur la seule variante : la chaîne vide est déjà la plus
/// petite. Mesuré, pas supposé — la mutation qui la remplace par un tri simple
/// ne fait tomber aucun test, et c'est normal.
///
/// Elle est conservée parce qu'elle **dit** l'intention, et que l'intention est
/// la seule chose qui protège ce tri d'une « simplification » un jour où
/// quelqu'un ajouterait une famille de variante nommée de façon à passer avant
/// la chaîne vide. Le commentaire vaut mieux que la surprise.
pub fn index_slots(slots: &[Slot]) -> IndexSlots {
    let mut index: IndexSlots = BTreeMap::new();
    for slot in slots {
        index
            .entry((slot.numero.clone(), normaliser_rarete(&slot.rarete)))
            .or_default()
            .push(slot.variante.clone());
    }
    for variantes in index.values_mut() {
        variantes
            .sort_by(|a, b| (usize::from(!a.is_empty()), a).cmp(&(usize::from(!b.is_empty()), b)));
    }
    index
}

/// Le libellé de rareté que la Set list emploie, par rareté normalisée.
///
/// Sert aux raretés que le classeur ne connaît pas encore : la ligne créée doit
/// bien porter un libellé, et à défaut d'orthographe locale c'est celle du wiki.
pub fn libelles(slots: &[Slot]) -> std::collections::HashMap<String, String> {
    let mut sortie = std::collections::HashMap::new();
    for slot in slots {
        sortie
            .entry(normaliser_rarete(&slot.rarete))
            .or_insert_with(|| slot.rarete.clone());
    }
    sortie
}

/// Suffixes de page à essayer, du plus probable au moins probable.
///
/// Portage de `_regions_candidates`.
pub fn regions_candidates(langue: &str) -> Vec<String> {
    let lg = if langue.is_empty() {
        "EN".to_owned()
    } else {
        langue.to_uppercase()
    };
    if REGIONS_OCG.contains(&lg.as_str()) {
        vec![
            format!("OCG-{lg}"),
            format!("TCG-{lg}"),
            "OCG-JP".to_owned(),
        ]
    } else {
        vec![format!("TCG-{lg}"), "TCG-EN".to_owned()]
    }
}

/// Choisit la page « Set Card Lists » qui correspond à la langue.
///
/// Portage de `choisir_page`. Le repli final est **le premier titre venu** :
/// une page d'une autre région vaut mieux que pas de structure du tout.
pub fn choisir_page<'a>(titres: &'a [String], langue: &str) -> Option<&'a str> {
    for region in regions_candidates(langue) {
        let marqueur = format!("({region})");
        if let Some(titre) = titres.iter().find(|t| t.contains(&marqueur)) {
            return Some(titre.as_str());
        }
    }
    titres.first().map(String::as_str)
}

/// Les pages « Set Card Lists » d'un **paquet bonus** : celui que Konami
/// vend avec un booster OCG, sous le même préfixe, numéroté à part
/// (`CORI-JPS10`, `ROTD-JPS01`). « +1 Bonus Pack » de Rise of the Duelist à
/// Alliance Insight (2020–2025), « +1 Expansion Pack » depuis Immortal Phoenix.
pub const PAGES_COMPLEMENTAIRES: [&str; 2] = [" +1 Bonus Pack", " +1 Expansion Pack"];

/// Le titre est-il celui d'un paquet bonus de `nom_set` ?
#[must_use]
pub fn est_page_complementaire(titre: &str, nom_set: &str) -> bool {
    PAGES_COMPLEMENTAIRES
        .iter()
        .any(|s| titre.starts_with(&format!("Set Card Lists:{nom_set}{s} (")))
}

/// La page **du set lui-même**, jamais celle de son paquet bonus.
///
/// `prefixsearch` sur « Set Card Lists:Chaos Origins » rend aussi « Chaos
/// Origins +1 Expansion Pack (OCG-JP) » : [`choisir_page`] pouvait la
/// prendre. Le titre exact d'abord, région par région ; à défaut, la règle
/// de [`choisir_page`] sur les seules pages qui ne sont pas un paquet bonus.
///
/// ```
/// use ygo_sources::yugipedia::structure::choisir_page_du_set;
/// let titres = vec![
///     "Set Card Lists:Chaos Origins +1 Expansion Pack (OCG-JP)".to_owned(),
///     "Set Card Lists:Chaos Origins (OCG-JP)".to_owned(),
/// ];
/// assert_eq!(
///     choisir_page_du_set(&titres, "Chaos Origins", "JP"),
///     Some("Set Card Lists:Chaos Origins (OCG-JP)")
/// );
/// ```
#[must_use]
pub fn choisir_page_du_set<'a>(
    titres: &'a [String],
    nom_set: &str,
    langue: &str,
) -> Option<&'a str> {
    for region in regions_candidates(langue) {
        let exact = format!("Set Card Lists:{nom_set} ({region})");
        if let Some(t) = titres.iter().find(|t| **t == exact) {
            return Some(t.as_str());
        }
    }
    let principales: Vec<&'a String> = titres
        .iter()
        .filter(|t| !est_page_complementaire(t, nom_set))
        .collect();
    for region in regions_candidates(langue) {
        let marqueur = format!("({region})");
        if let Some(t) = principales.iter().find(|t| t.contains(&marqueur)) {
            return Some(t.as_str());
        }
    }
    principales.first().map(|t| t.as_str())
}

/// Les pages des paquets bonus de `nom_set`, dans la région de la page
/// principale retenue — seulement celles que `titres` contient : aucune
/// requête n'est faite pour une page qui n'existe pas.
///
/// ```
/// use ygo_sources::yugipedia::structure::pages_complementaires;
/// let titres = vec![
///     "Set Card Lists:Chaos Origins (OCG-JP)".to_owned(),
///     "Set Card Lists:Chaos Origins +1 Expansion Pack (OCG-JP)".to_owned(),
///     "Set Card Lists:Chaos Origins +1 Expansion Pack (OCG-KR)".to_owned(),
/// ];
/// assert_eq!(
///     pages_complementaires(&titres, "Chaos Origins", "Set Card Lists:Chaos Origins (OCG-JP)"),
///     vec!["Set Card Lists:Chaos Origins +1 Expansion Pack (OCG-JP)".to_owned()]
/// );
/// ```
#[must_use]
pub fn pages_complementaires(
    titres: &[String],
    nom_set: &str,
    page_principale: &str,
) -> Vec<String> {
    let Some((_, region)) = page_principale.rsplit_once('(') else {
        return Vec::new();
    };
    let region = region.trim_end_matches(')');
    PAGES_COMPLEMENTAIRES
        .iter()
        .map(|s| format!("Set Card Lists:{nom_set}{s} ({region})"))
        .filter(|t| titres.contains(t))
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    fn entree(numero: &str, raretes: &[&str], extended_art: bool, note: &str) -> EntreeSetList {
        EntreeSetList {
            numero: numero.to_owned(),
            nom: "Dark Magician".to_owned(),
            raretes: raretes.iter().map(|r| (*r).to_owned()).collect(),
            extended_art,
            note: note.to_owned(),
        }
    }

    #[test]
    fn la_normalisation_encaisse_les_coquilles_du_wiki() {
        // Relevés tels quels sur RA05.
        assert_eq!(
            normaliser_rarete("PLatinum Secret Rare"),
            "platinumsecretrare"
        );
        assert_eq!(
            normaliser_rarete("Quarter Century Secret Rare"),
            "quartercenturysecretrare"
        );
        assert_eq!(normaliser_rarete("Ultra  Rare"), "ultrarare");
        assert_eq!(normaliser_rarete("Collector's Rare"), "collectorsrare");
    }

    #[test]
    fn le_drapeau_du_parser_prime_sur_la_note() {
        let e = entree("A-EN001", &["Ultra Rare"], true, "");
        assert_eq!(variante_depuis_entree(&e), "EA");
    }

    #[test]
    fn les_formulations_des_sets_reels_donnent_toutes_aa() {
        for texte in [
            "stamp artwork",
            "alternate art",
            "alternate artwork",
            "new artwork",
            "3rd artwork",
            "2nd, 5th and 6th artwork",
        ] {
            let note = format!("description::({texte})");
            let e = entree("A-EN001", &["Ultra Rare"], false, &note);
            assert_eq!(variante_depuis_entree(&e), "AA", "annotation : {texte}");
        }
    }

    #[test]
    fn extended_art_dans_la_note_donne_ea_et_non_aa() {
        let e = entree(
            "A-EN001",
            &["Ultra Rare"],
            false,
            "description::(extended artwork)",
        );
        assert_eq!(variante_depuis_entree(&e), "EA", "EA est testé avant AA");
    }

    #[test]
    fn une_annotation_de_mot_de_passe_l_emporte_sur_tout_le_reste() {
        // « alternate password » : même illustration, autre code Konami —
        // relevé sur RA03.
        let seul = entree(
            "A-EN001",
            &["Ultra Rare"],
            false,
            "description::(alternate password)",
        );
        assert_eq!(variante_depuis_entree(&seul), "");

        // Ce cas-là seul ne prouve rien : « alternate password » ne contient
        // pas « art », donc il tomberait de toute façon. Ce que le garde-fou
        // fait vraiment, c'est PRIMER — y compris sur « extended art », qui est
        // testé après lui.
        let melange = entree(
            "A-EN001",
            &["Ultra Rare"],
            false,
            "description::(alternate password, extended art)",
        );
        assert_eq!(
            variante_depuis_entree(&melange),
            "",
            "le mot de passe est examiné avant les variantes"
        );
    }

    #[test]
    fn une_note_sans_description_ne_donne_rien() {
        let e = entree("A-EN001", &["Ultra Rare"], false, "artwork");
        assert_eq!(
            variante_depuis_entree(&e),
            "",
            "le mot seul ne suffit pas — il faut l'annotation"
        );
    }

    #[test]
    fn la_description_s_arrete_a_la_premiere_parenthese() {
        let e = entree(
            "A-EN001",
            &["Ultra Rare"],
            false,
            "description::(password) (artwork)",
        );
        assert_eq!(
            variante_depuis_entree(&e),
            "",
            "le second groupe n'est pas la description"
        );
    }

    #[test]
    fn une_entree_donne_un_slot_par_rarete() {
        let slots = slots(&[entree(
            "locr-jp001 ",
            &["Ultra Rare", "Secret Rare", ""],
            false,
            "",
        )]);
        assert_eq!(slots.len(), 2, "la rareté vide est écartée");
        assert_eq!(slots[0].numero, "LOCR-JP001", "numéro en majuscules");
    }

    /// Les annonces de `LCKC` (2026-10-16), et ce qui n'en est pas une.
    #[test]
    fn les_illustrations_d_un_meme_tirage_se_comptent() {
        let compte = |note: &str| {
            nombre_d_illustrations(&entree("LCKC-EN001", &["Ultra Rare"], false, note))
        };
        assert_eq!(compte("description::(four unique artworks)"), 4);
        assert_eq!(compte("description::(two unique artworks)"), 2);
        assert_eq!(compte("description::(Three Different Artworks)"), 3);
        assert_eq!(compte("description::(2 artworks)"), 2);
        // Un ordinal désigne UNE illustration de la carte, pas un nombre.
        assert_eq!(compte("description::(6th artwork)"), 1);
        assert_eq!(compte("description::(2nd, 5th and 6th artwork)"), 1);
        assert_eq!(compte("description::(alternate art)"), 1);
        assert_eq!(compte("description::(new artworks)"), 1);
        assert_eq!(compte("description::(one artworks)"), 1, "un seul : rien");
        assert_eq!(compte("description::(99 artworks)"), 1, "une coquille");
        assert_eq!(compte("four unique artworks"), 1, "sans description");
        assert_eq!(compte(""), 1);
    }

    #[test]
    fn un_tirage_en_quatre_illustrations_donne_quatre_slots() {
        let s = slots(&[
            entree(
                "LCKC-EN001",
                &["Ultra Rare"],
                false,
                "description::(four unique artworks)",
            ),
            entree("LCKC-EN002", &["Ultra Rare"], false, ""),
            entree(
                "LCKC-EN046",
                &["Ultra Rare", "Secret Rare"],
                false,
                "description::(two unique artworks)",
            ),
        ]);
        let index = index_slots(&s);
        let variantes = |numero: &str, rarete: &str| {
            index
                .get(&(numero.to_owned(), rarete.to_owned()))
                .cloned()
                .unwrap_or_default()
        };
        assert_eq!(variantes("LCKC-EN001", "ultrarare"), ["AA"; 4]);
        assert_eq!(variantes("LCKC-EN002", "ultrarare"), [""]);
        assert_eq!(variantes("LCKC-EN046", "ultrarare"), ["AA"; 2]);
        assert_eq!(
            variantes("LCKC-EN046", "secretrare"),
            ["AA"; 2],
            "chaque rareté du tirage"
        );
        assert_eq!(s.len(), 4 + 1 + 2 + 2);
    }

    #[test]
    fn une_entree_sans_variante_n_est_jamais_demultipliee() {
        // « password » écarte la variante : sans annotation d'illustration,
        // rien ne dit laquelle chaque tirage montrerait.
        let s = slots(&[entree(
            "A-EN001",
            &["Ultra Rare"],
            false,
            "description::(two unique artworks, alternate password)",
        )]);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].variante, "");
    }

    #[test]
    fn une_entree_sans_numero_ne_donne_aucun_slot() {
        assert!(slots(&[entree("  ", &["Ultra Rare"], false, "")]).is_empty());
    }

    #[test]
    fn l_index_range_les_normales_avant_les_variantes() {
        let slots = slots(&[
            entree("A-EN001", &["Ultra Rare"], true, ""),
            entree("A-EN001", &["Ultra Rare"], false, ""),
            entree(
                "A-EN001",
                &["Ultra Rare"],
                false,
                "description::(stamp artwork)",
            ),
        ]);
        let index = index_slots(&slots);
        let variantes = index
            .get(&("A-EN001".to_owned(), "ultrarare".to_owned()))
            .unwrap();
        assert_eq!(
            variantes,
            &["", "AA", "EA"],
            "normale d'abord, puis l'ordre alphabétique des variantes"
        );
    }

    #[test]
    fn le_libelle_retenu_est_le_premier_rencontre() {
        let slots = slots(&[
            entree("A-EN001", &["Ultra Rare"], false, ""),
            entree("A-EN002", &["ULTRA RARE"], false, ""),
        ]);
        assert_eq!(libelles(&slots)["ultrarare"], "Ultra Rare");
    }

    #[test]
    fn une_langue_ocg_essaie_sa_region_puis_le_japonais() {
        assert_eq!(regions_candidates("JP"), ["OCG-JP", "TCG-JP", "OCG-JP"]);
        assert_eq!(regions_candidates("KR"), ["OCG-KR", "TCG-KR", "OCG-JP"]);
        assert_eq!(regions_candidates("FR"), ["TCG-FR", "TCG-EN"]);
        assert_eq!(regions_candidates(""), ["TCG-EN", "TCG-EN"]);
    }

    #[test]
    fn la_page_choisie_est_celle_de_la_region_puis_n_importe_laquelle() {
        let titres = vec![
            "Set Card Lists:Le Set (TCG-EN)".to_owned(),
            "Set Card Lists:Le Set (OCG-JP)".to_owned(),
        ];
        assert_eq!(choisir_page(&titres, "JP"), Some(titres[1].as_str()));
        assert_eq!(choisir_page(&titres, "EN"), Some(titres[0].as_str()));

        let exotique = vec!["Set Card Lists:Le Set (OCG-KR)".to_owned()];
        assert_eq!(
            choisir_page(&exotique, "EN"),
            Some(exotique[0].as_str()),
            "aucune région ne colle : le premier titre vaut mieux que rien"
        );
        assert_eq!(choisir_page(&[], "EN"), None);
    }
}
