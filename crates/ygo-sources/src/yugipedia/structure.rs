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

/// Développe les entrées d'une Set list en slots.
///
/// Portage de la seconde moitié de `charger_structure` — celle qui ne touche
/// pas au réseau. Une entrée sans numéro est ignorée ; une rareté vide aussi.
pub fn slots(entrees: &[EntreeSetList]) -> Vec<Slot> {
    let mut sortie = Vec::new();
    for entree in entrees {
        let numero = entree.numero.trim().to_uppercase();
        if numero.is_empty() {
            continue;
        }
        let variante = variante_depuis_entree(entree);
        for rarete in &entree.raretes {
            let rarete = rarete.trim();
            if rarete.is_empty() {
                continue;
            }
            sortie.push(Slot {
                numero: numero.clone(),
                nom: entree.nom.clone(),
                rarete: rarete.to_owned(),
                variante: variante.to_owned(),
            });
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
