// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Référentiel des raretés — codes Scanflip, noms français, anglais, alias.
//!
//! Portage de `module/gestion_rarete/raretes_reference.py`.
//!
//! # La table est extraite, pas retranscrite
//!
//! `assets/raretes_reference.json` est produit par `outils/oracle_artwork.py`,
//! qui **importe le vrai module Python** et sérialise son dictionnaire
//! `RARETES` dans son ordre d'insertion. Aucune ligne n'a été recopiée à la
//! main : quarante-quatre codes, quatre-vingt-huit libellés et onze alias
//! auraient donné toutes leurs chances aux coquilles.
//!
//! # L'ordre compte, et ce n'est pas un détail de forme
//!
//! Quatorze clés normalisées sont **en collision** — `grandmasterrare` en
//! désigne trois à elle seule. Le Python construit son index en parcourant le
//! dictionnaire dans l'ordre d'insertion, et **la dernière entrée écrase les
//! précédentes**. Un index bâti dans un autre ordre — alphabétique, par
//! exemple — rendrait d'autres codes pour ces quatorze clés. D'où la liste
//! ordonnée plutôt qu'un objet JSON, et d'où le `Vec` plutôt qu'une table.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::Deserialize;

/// La table, telle qu'extraite du Python.
const TABLE_JSON: &str = include_str!("../../../../assets/raretes_reference.json");

/// Une entrée du référentiel.
#[derive(Debug, Clone, Deserialize)]
pub struct Rarete {
    /// Code Scanflip (`UR`, `ScR`…).
    pub code: String,
    /// Libellé français.
    pub fr: String,
    /// Libellé anglais.
    pub en: String,
    /// Autres orthographes acceptées, alias YGOPRODeck compris.
    #[serde(default)]
    pub alias: Vec<String>,
}

struct Index {
    table: Vec<Rarete>,
    par_code: HashMap<String, usize>,
    par_fr: HashMap<String, usize>,
    par_en: HashMap<String, usize>,
    par_normalise: HashMap<String, usize>,
}

fn index() -> &'static Index {
    static INDEX: OnceLock<Index> = OnceLock::new();
    INDEX.get_or_init(|| {
        let table: Vec<Rarete> = serde_json::from_str(TABLE_JSON).unwrap_or_default();
        let mut par_code = HashMap::new();
        let mut par_fr = HashMap::new();
        let mut par_en = HashMap::new();
        let mut par_normalise = HashMap::new();
        // Ordre d'insertion : sur clé normalisée, la dernière entrée gagne.
        for (i, r) in table.iter().enumerate() {
            par_code.insert(r.code.clone(), i);
            par_fr.insert(r.fr.clone(), i);
            par_en.insert(r.en.clone(), i);
            par_normalise.insert(normaliser(&r.fr), i);
            par_normalise.insert(normaliser(&r.en), i);
            for alias in &r.alias {
                par_normalise.insert(normaliser(alias), i);
            }
        }
        Index {
            table,
            par_code,
            par_fr,
            par_en,
            par_normalise,
        }
    })
}

/// Toutes les raretés du référentiel, dans l'ordre de la table.
pub fn table() -> &'static [Rarete] {
    &index().table
}

/// Normalise un libellé pour une comparaison tolérante.
///
/// Portage de `_normalize` : accents courants remplacés, puis minuscules,
/// puis suppression des espaces, apostrophes et tirets.
///
/// # Une asymétrie à reproduire
///
/// La table d'accents du Python ne porte que trois majuscules — `É`, `È`, `Ê`.
/// Les autres majuscules accentuées (`À`, `Ç`, `Ô`…) traversent donc la
/// translation intactes, et la mise en minuscules qui suit les transforme en
/// `à`, `ç`, `ô` — **qui ne sont plus remplacés**, la translation étant déjà
/// passée. `Ça` donne `ça` là où `ça` donne `ca`. Ce n'est vraisemblablement
/// pas voulu, mais c'est le comportement en place.
pub fn normaliser(texte: &str) -> String {
    if texte.is_empty() {
        return String::new();
    }
    let translitere: String = texte.chars().map(accent_simplifie).collect();
    translitere
        .to_lowercase()
        .chars()
        .filter(|c| *c != ' ' && *c != '\'' && *c != '-')
        .collect()
}

/// La table d'accents du Python, caractère par caractère.
fn accent_simplifie(c: char) -> char {
    match c {
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'à' | 'â' | 'ä' => 'a',
        'î' | 'ï' => 'i',
        'ô' | 'ö' => 'o',
        'ù' | 'û' | 'ü' => 'u',
        'ç' => 'c',
        'É' | 'È' | 'Ê' => 'E',
        autre => autre,
    }
}

/// Code Scanflip d'un libellé — français, anglais ou alias.
///
/// Portage de `name_to_code`. Trois passes, dans l'ordre : correspondance
/// exacte française, exacte anglaise, puis normalisée. `None` quand rien ne
/// correspond — c'est à l'appelant de décider du repli, et tous ne font pas le
/// même.
pub fn nom_vers_code(nom: &str) -> Option<&'static str> {
    if nom.is_empty() {
        return None;
    }
    let idx = index();
    let i = idx
        .par_fr
        .get(nom)
        .or_else(|| idx.par_en.get(nom))
        .or_else(|| idx.par_normalise.get(&normaliser(nom)))?;
    idx.table.get(*i).map(|r| r.code.as_str())
}

/// Libellé anglais d'un code — **le code lui-même** s'il est inconnu.
pub fn code_vers_nom_en(code: &str) -> String {
    match index()
        .par_code
        .get(code)
        .and_then(|i| index().table.get(*i))
    {
        Some(r) => r.en.clone(),
        None => code.to_owned(),
    }
}

/// Libellé français d'un code — **le code lui-même** s'il est inconnu.
pub fn code_vers_nom_fr(code: &str) -> String {
    match index()
        .par_code
        .get(code)
        .and_then(|i| index().table.get(*i))
    {
        Some(r) => r.fr.clone(),
        None => code.to_owned(),
    }
}

/// Le code figure-t-il au référentiel ?
pub fn code_connu(code: &str) -> bool {
    index().par_code.contains_key(code)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    #[test]
    fn la_table_extraite_se_charge() {
        assert_eq!(table().len(), 44, "44 codes dans le référentiel réel");
        let alias: usize = table().iter().map(|r| r.alias.len()).sum();
        assert_eq!(alias, 11);
        assert_eq!(table()[0].code, "C");
        assert_eq!(table()[0].en, "Common");
    }

    #[test]
    fn les_trois_passes_de_correspondance() {
        assert_eq!(nom_vers_code("Commune"), Some("C"));
        assert_eq!(nom_vers_code("Common"), Some("C"));
        assert_eq!(nom_vers_code("COMMON"), Some("C"));
        assert_eq!(nom_vers_code("com-mon"), Some("C"));
    }

    #[test]
    fn les_alias_sont_reconnus() {
        // « Short Print » est un alias de Common : il n'a pas de code à lui.
        assert_eq!(nom_vers_code("Short Print"), Some("C"));
        assert_eq!(code_vers_nom_en("C"), "Common");
    }

    #[test]
    fn un_code_inconnu_se_rend_lui_meme() {
        assert_eq!(code_vers_nom_en("ZZZ"), "ZZZ");
        assert_eq!(code_vers_nom_fr("ZZZ"), "ZZZ");
        assert!(!code_connu("ZZZ"));
        // Les codes du référentiel sont ceux de Scanflip — « U » pour l'Ultra
        // Rare, pas l'abréviation Yugipedia « UR », qui vit ailleurs.
        assert!(code_connu("U"));
        assert!(!code_connu("UR"));
        assert_eq!(nom_vers_code("Ultra Rare"), Some("U"));
    }

    #[test]
    fn normalisation_asymetrique_sur_les_majuscules_accentuees() {
        assert_eq!(normaliser("Commune Parallèle"), "communeparallele");
        assert_eq!(normaliser("Ça"), "ça");
        assert_eq!(normaliser("ça"), "ca");
        assert_eq!(normaliser("Élégant"), "elegant");
        assert_eq!(normaliser("Collector's Rare"), "collectorsrare");
        assert_eq!(normaliser(""), "");
    }

    #[test]
    fn rien_ne_correspond_a_rien() {
        assert_eq!(nom_vers_code(""), None);
        assert_eq!(nom_vers_code("Rareté inconnue"), None);
    }
}
