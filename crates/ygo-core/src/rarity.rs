// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Priorités de rareté — `bdd/rarity_config.json`.
//!
//! Portage de `module/gestion_rarete/gestion_rarete_service.py`.
//!
//! Une priorité est un entier : **plus il est élevé, plus la rareté est rare**.
//! C'est l'entrée directe du tri des cartes et du filtre « N raretés par
//! artwork ».
//!
//! # Deux défauts divergents, à reproduire tels quels
//!
//! Quand une rareté est absente de `rarity_config.json`, le Python ne lui
//! donne pas la même valeur selon l'appelant :
//!
//! - `tri_carte.sort_cartes` utilise [`PRIORITE_INCONNUE_TRI`] (`9999`), ce qui
//!   place la rareté inconnue **en dernier** au tri croissant ;
//! - `tri_carte.filtrer_n_raretes_par_artwork` utilise
//!   [`PRIORITE_INCONNUE_FILTRE`] (`0`), ce qui en fait la **moins rare** au
//!   filtrage.
//!
//! Ce n'est vraisemblablement pas voulu, et cela se constate sur les données
//! réelles : 80 lignes portent `PLatinum Secret Rare` (avec un L majuscule)
//! quand `rarity_config.json` ne connaît que `Platinum Secret Rare`. Le portage
//! étant iso-fonctionnel, **on reproduit**. La normalisation est au backlog.

pub mod canon;
pub mod reference;
pub mod scanflip;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::{CoreError, Result};

/// Priorité attribuée à une rareté inconnue lors du **tri**
/// (`tri_carte._build_sort_key_function`). La place en dernier.
pub const PRIORITE_INCONNUE_TRI: i64 = 9999;

/// Priorité attribuée à une rareté inconnue lors du **filtrage**
/// (`tri_carte.filtrer_n_raretes_par_artwork`). La rend la moins rare.
pub const PRIORITE_INCONNUE_FILTRE: i64 = 0;

/// Ordre de référence servant à générer des priorités par défaut
/// (`gestion_rarete_service._DEFAULT_ORDER`).
///
/// La priorité d'une rareté connue est son **index + 1**. Les raretés absentes
/// de cette liste sont classées après, par ordre alphabétique, à partir de
/// `ORDRE_DEFAUT.len() + 1`.
pub const ORDRE_DEFAUT: [&str; 36] = [
    "Common",
    "Rare",
    "Super Rare",
    "Ultra Rare",
    "Secret Rare",
    "Platinum Rare",
    "Platinum Secret Rare",
    "Ultra Secret Rare",
    "Collector's Rare",
    "Prismatic Secret Rare",
    "Ultimate Rare",
    "Quarter Century Secret Rare",
    "Short Print",
    "Super Short Print",
    "Extra Secret Rare",
    "Gold Rare",
    "Gold Secret Rare",
    "Premium Gold Rare",
    "Ghost Rare",
    "Ghost/Gold Rare",
    "Starlight Rare",
    "Mosaic Rare",
    "Shatterfoil Rare",
    "Duel Terminal Normal Parallel Rare",
    "Duel Terminal Rare Parallel Rare",
    "Duel Terminal Super Parallel Rare",
    "Duel Terminal Ultra Parallel Rare",
    "Normal Parallel Rare",
    "Super Parallel Rare",
    "Ultra Parallel Rare",
    "10000 Secret Rare",
    "Duel Terminal Normal Rare Parallel Rare",
    "Extra Secret",
    "Starfoil",
    "Starfoil Rare",
    "Ultra Rare (Pharaoh's Rare)",
];

/// Liste de repli quand `cardinfo.db` est absente ou illisible
/// (`gestion_rarete_service._RARITIES_FALLBACK`) — les 21 premières entrées de
/// [`ORDRE_DEFAUT`].
pub const RARETES_REPLI: [&str; 21] = [
    "Common",
    "Rare",
    "Super Rare",
    "Ultra Rare",
    "Secret Rare",
    "Platinum Rare",
    "Platinum Secret Rare",
    "Ultra Secret Rare",
    "Collector's Rare",
    "Prismatic Secret Rare",
    "Ultimate Rare",
    "Quarter Century Secret Rare",
    "Short Print",
    "Super Short Print",
    "Extra Secret Rare",
    "Gold Rare",
    "Gold Secret Rare",
    "Premium Gold Rare",
    "Ghost Rare",
    "Ghost/Gold Rare",
    "Starlight Rare",
];

/// Table des priorités de rareté chargée depuis `rarity_config.json`.
#[derive(Debug, Clone, Default)]
pub struct Priorites {
    table: BTreeMap<String, i64>,
}

impl Priorites {
    /// Charge `rarity_config.json`.
    ///
    /// Fichier absent, illisible ou JSON invalide donnent une table **vide** —
    /// c'est exactement le comportement de `load_rarity_priorities()`, et non
    /// un repli sur des priorités par défaut. Toutes les raretés tombent alors
    /// sur leur valeur d'inconnue.
    pub fn charger(chemin: impl AsRef<Path>) -> Self {
        let chemin = chemin.as_ref();
        let Ok(texte) = std::fs::read_to_string(chemin) else {
            return Self::default();
        };
        match serde_json::from_str::<BTreeMap<String, i64>>(&texte) {
            Ok(table) => Self { table },
            Err(e) => {
                tracing::warn!(
                    chemin = %chemin.display(),
                    erreur = %e,
                    "rarity_config.json illisible — aucune priorité chargée"
                );
                Self::default()
            }
        }
    }

    /// Construit une table à partir de paires nom → priorité.
    pub fn depuis_paires<I, S>(paires: I) -> Self
    where
        I: IntoIterator<Item = (S, i64)>,
        S: Into<String>,
    {
        Self {
            table: paires.into_iter().map(|(k, v)| (k.into(), v)).collect(),
        }
    }

    /// Priorité d'une rareté pour le **tri** — `9999` si inconnue.
    pub fn pour_tri(&self, rarete: &str) -> i64 {
        self.table
            .get(rarete)
            .copied()
            .unwrap_or(PRIORITE_INCONNUE_TRI)
    }

    /// Priorité d'une rareté pour le **filtrage** — `0` si inconnue.
    pub fn pour_filtre(&self, rarete: &str) -> i64 {
        self.table
            .get(rarete)
            .copied()
            .unwrap_or(PRIORITE_INCONNUE_FILTRE)
    }

    /// Priorité déclarée, ou `None` si la rareté est absente de la table.
    pub fn brute(&self, rarete: &str) -> Option<i64> {
        self.table.get(rarete).copied()
    }

    /// Nombre de raretés déclarées.
    pub fn len(&self) -> usize {
        self.table.len()
    }

    /// La table est-elle vide ?
    pub fn is_empty(&self) -> bool {
        self.table.is_empty()
    }

    /// Itère sur les paires (rareté, priorité), triées par nom de rareté.
    pub fn iter(&self) -> impl Iterator<Item = (&str, i64)> {
        self.table.iter().map(|(k, v)| (k.as_str(), *v))
    }

    /// Écrit la table dans `rarity_config.json`.
    ///
    /// Format repris du Python : indentation de 4 espaces, caractères non-ASCII
    /// conservés tels quels (`ensure_ascii=False`).
    pub fn enregistrer(&self, chemin: impl AsRef<Path>) -> Result<()> {
        let chemin: PathBuf = chemin.as_ref().to_path_buf();
        if let Some(parent) = chemin.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| CoreError::io(parent, e))?;
            }
        }
        let mut sortie = Vec::new();
        let indent = b"    ";
        let formateur = serde_json::ser::PrettyFormatter::with_indent(indent);
        let mut ser = serde_json::Serializer::with_formatter(&mut sortie, formateur);
        serde::Serialize::serialize(&self.table, &mut ser)
            .map_err(|e| CoreError::json(&chemin, e))?;
        std::fs::write(&chemin, sortie).map_err(|e| CoreError::io(&chemin, e))
    }
}

/// Génère des priorités par défaut pour une liste de raretés.
///
/// Portage **pur** de `gestion_rarete_service.get_default_priorities()` : la
/// lecture des raretés distinctes de `set_prints` reste du ressort de `ygo-db`,
/// qui appelle cette fonction avec la liste obtenue. C'est la règle R1 —
/// la logique ici ne connaît ni SQLite ni le réseau.
///
/// - Une rareté présente dans [`ORDRE_DEFAUT`] reçoit `index + 1`.
/// - Les autres, triées par ordre alphabétique, reçoivent des rangs
///   consécutifs à partir de `ORDRE_DEFAUT.len() + 1` (soit 37).
pub fn priorites_par_defaut<S: AsRef<str>>(raretes: &[S]) -> Priorites {
    let mut table = BTreeMap::new();
    let mut inconnues: Vec<&str> = Vec::new();

    for rarete in raretes {
        let nom = rarete.as_ref();
        match ORDRE_DEFAUT.iter().position(|r| *r == nom) {
            Some(index) => {
                table.insert(nom.to_owned(), index as i64 + 1);
            }
            None => inconnues.push(nom),
        }
    }

    inconnues.sort_unstable();
    inconnues.dedup();
    let premier_rang = ORDRE_DEFAUT.len() as i64 + 1;
    for (decalage, nom) in inconnues.into_iter().enumerate() {
        table.insert(nom.to_owned(), premier_rang + decalage as i64);
    }

    Priorites { table }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    /// Le `rarity_config.json` réel de l'utilisateur, tel qu'il est sur disque.
    const REFERENCE: &str = include_str!("../../../tests/fixtures/rarity_config.reference.json");

    #[test]
    fn fichier_absent_donne_une_table_vide() {
        let p = Priorites::charger("/chemin/qui/n/existe/pas.json");
        assert!(p.is_empty());
        // Conséquence : les deux défauts divergents s'appliquent.
        assert_eq!(p.pour_tri("Ultra Rare"), PRIORITE_INCONNUE_TRI);
        assert_eq!(p.pour_filtre("Ultra Rare"), PRIORITE_INCONNUE_FILTRE);
    }

    #[test]
    fn fichier_corrompu_donne_une_table_vide() {
        let tmp = tempfile::tempdir().unwrap();
        let chemin = tmp.path().join("rarity_config.json");
        std::fs::write(&chemin, b"[1, 2, 3]").unwrap();
        assert!(Priorites::charger(&chemin).is_empty());
    }

    #[test]
    fn charge_le_fichier_reel_de_l_utilisateur() {
        let tmp = tempfile::tempdir().unwrap();
        let chemin = tmp.path().join("rarity_config.json");
        std::fs::write(&chemin, REFERENCE).unwrap();

        let p = Priorites::charger(&chemin);
        assert_eq!(p.len(), 43, "43 raretés dans le fichier réel de la V1.0.3");
        assert_eq!(p.brute("Common"), Some(1));
        assert_eq!(p.brute("Ultra Rare"), Some(4));
        assert_eq!(p.brute("Quarter Century Secret Rare"), Some(12));
        assert_eq!(p.brute("Grand Master Rare"), Some(50));
    }

    /// Bug latent observé dans les données réelles : 80 lignes de classeur
    /// portent `PLatinum Secret Rare` (deuxième lettre en majuscule), qui
    /// n'existe pas dans `rarity_config.json`. À reproduire, pas à corriger.
    #[test]
    fn platinum_mal_orthographie_reste_inconnu() {
        let tmp = tempfile::tempdir().unwrap();
        let chemin = tmp.path().join("rarity_config.json");
        std::fs::write(&chemin, REFERENCE).unwrap();
        let p = Priorites::charger(&chemin);

        assert_eq!(p.brute("Platinum Secret Rare"), Some(7));
        assert_eq!(p.brute("PLatinum Secret Rare"), None);
        assert_eq!(p.pour_tri("PLatinum Secret Rare"), 9999);
        assert_eq!(p.pour_filtre("PLatinum Secret Rare"), 0);
    }

    #[test]
    fn priorites_par_defaut_suit_l_ordre_de_reference() {
        let raretes = ["Ultra Rare", "Common", "Secret Rare"];
        let p = priorites_par_defaut(&raretes);
        assert_eq!(p.brute("Common"), Some(1));
        assert_eq!(p.brute("Ultra Rare"), Some(4));
        assert_eq!(p.brute("Secret Rare"), Some(5));
    }

    #[test]
    fn priorites_par_defaut_classe_les_inconnues_apres_par_ordre_alphabetique() {
        let raretes = ["Zebra Rare", "Common", "Alpha Rare"];
        let p = priorites_par_defaut(&raretes);
        assert_eq!(p.brute("Common"), Some(1));
        assert_eq!(p.brute("Alpha Rare"), Some(37), "len(ORDRE_DEFAUT) + 1");
        assert_eq!(p.brute("Zebra Rare"), Some(38));
    }

    #[test]
    fn aller_retour_disque() {
        let tmp = tempfile::tempdir().unwrap();
        let chemin = tmp.path().join("sous-dossier").join("rarity_config.json");

        let origine = Priorites::depuis_paires([("Common", 1), ("Ultra Rare", 4)]);
        origine.enregistrer(&chemin).unwrap();

        let relu = Priorites::charger(&chemin);
        assert_eq!(relu.brute("Common"), Some(1));
        assert_eq!(relu.brute("Ultra Rare"), Some(4));
        assert_eq!(relu.len(), 2);
    }

    #[test]
    fn les_constantes_de_portage_ont_la_bonne_taille() {
        assert_eq!(ORDRE_DEFAUT.len(), 36);
        assert_eq!(RARETES_REPLI.len(), 21);
        // RARETES_REPLI doit être le préfixe exact d'ORDRE_DEFAUT.
        for (i, r) in RARETES_REPLI.iter().enumerate() {
            assert_eq!(ORDRE_DEFAUT.get(i), Some(r));
        }
    }
}
