// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Le journal des téléchargements en cours — `bdd/downloads_actifs.json`.
//!
//! Portage de la persistance légère de `file_attente_classeur`.
//!
//! # Ce qu'il sert à faire, et lui seul
//!
//! Un code y entre quand son téléchargement commence, et en sort quand il se
//! termine — **succès, erreur ou annulation confondus**. Si l'application est
//! fermée en cours de route, les codes restés là sont repris au démarrage
//! suivant.
//!
//! Il ne mémorise **pas** ce qui a été téléchargé. C'est délibéré : la reprise
//! rescanne le disque, et ne redemande que ce qui manque encore. Un journal
//! détaillé serait une seconde source de vérité à tenir d'accord avec la
//! première, et il finirait par mentir.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Ce que sait faire un journal de téléchargements.
///
/// L'abstraction existe pour que la file d'attente se teste sans disque.
pub trait Journal {
    /// Les codes dont le téléchargement était en cours.
    fn charger(&self) -> Vec<String>;
    /// Note qu'un téléchargement commence.
    fn ajouter(&self, code: &str);
    /// Note qu'un téléchargement s'achève, quelle qu'en soit l'issue.
    fn retirer(&self, code: &str);
}

/// Le journal sur disque.
#[derive(Debug, Clone)]
pub struct JournalDisque {
    chemin: PathBuf,
}

impl JournalDisque {
    /// Ouvre — ou prépare — le journal à ce chemin.
    #[must_use]
    pub fn nouveau(chemin: impl Into<PathBuf>) -> Self {
        Self {
            chemin: chemin.into(),
        }
    }

    /// Le chemin du fichier.
    #[must_use]
    pub fn chemin(&self) -> &Path {
        &self.chemin
    }

    fn lire(&self) -> BTreeSet<String> {
        let Ok(texte) = std::fs::read_to_string(&self.chemin) else {
            return BTreeSet::new();
        };
        match serde_json::from_str::<Vec<String>>(&texte) {
            Ok(codes) => codes
                .into_iter()
                .map(|c| c.trim().to_uppercase())
                .filter(|c| !c.is_empty())
                .collect(),
            Err(e) => {
                tracing::warn!(
                    chemin = %self.chemin.display(),
                    erreur = %e,
                    "downloads_actifs.json illisible — journal ignoré"
                );
                BTreeSet::new()
            }
        }
    }

    /// Écrit le journal, **atomiquement**.
    ///
    /// Même raison que pour les images : un journal à moitié écrit, relu au
    /// démarrage suivant, ferait perdre des reprises sans rien signaler.
    fn ecrire(&self, codes: &BTreeSet<String>) {
        let liste: Vec<&String> = codes.iter().collect();
        let Ok(texte) = serde_json::to_string(&liste) else {
            return;
        };
        if let Err(e) = crate::telechargement::ecrire_atomique(&self.chemin, texte.as_bytes()) {
            tracing::warn!(
                chemin = %self.chemin.display(),
                erreur = %e,
                "journal des téléchargements non écrit"
            );
        }
    }
}

impl Journal for JournalDisque {
    fn charger(&self) -> Vec<String> {
        self.lire().into_iter().collect()
    }

    fn ajouter(&self, code: &str) {
        let mut codes = self.lire();
        codes.insert(code.trim().to_uppercase());
        self.ecrire(&codes);
    }

    fn retirer(&self, code: &str) {
        let mut codes = self.lire();
        codes.remove(&code.trim().to_uppercase());
        self.ecrire(&codes);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    fn journal() -> (tempfile::TempDir, JournalDisque) {
        let tmp = tempfile::tempdir().unwrap();
        let j = JournalDisque::nouveau(tmp.path().join("bdd").join("downloads_actifs.json"));
        (tmp, j)
    }

    #[test]
    fn un_journal_absent_est_vide_et_ne_gene_pas() {
        let (_tmp, j) = journal();
        assert!(j.charger().is_empty());
        // Retirer d'un journal inexistant ne doit rien casser.
        j.retirer("RA02");
        assert!(j.charger().is_empty());
    }

    #[test]
    fn un_aller_retour_conserve_les_codes() {
        let (_tmp, j) = journal();
        j.ajouter("ra02");
        j.ajouter("LDK2");
        assert_eq!(j.charger(), ["LDK2", "RA02"], "normalisés et triés");
        j.retirer("RA02");
        assert_eq!(j.charger(), ["LDK2"]);
    }

    #[test]
    fn le_meme_code_n_entre_qu_une_fois() {
        let (_tmp, j) = journal();
        j.ajouter("RA02");
        j.ajouter("RA02");
        j.ajouter(" ra02 ");
        assert_eq!(j.charger(), ["RA02"]);
    }

    #[test]
    fn un_journal_corrompu_est_ignore_sans_lever() {
        let (_tmp, j) = journal();
        std::fs::create_dir_all(j.chemin().parent().unwrap()).unwrap();
        std::fs::write(j.chemin(), b"{ pas une liste }").unwrap();
        assert!(j.charger().is_empty());
        // Et il reste écrivable.
        j.ajouter("RA02");
        assert_eq!(j.charger(), ["RA02"]);
    }

    #[test]
    fn le_dossier_parent_est_cree_au_besoin() {
        let (_tmp, j) = journal();
        assert!(!j.chemin().parent().unwrap().exists());
        j.ajouter("RA02");
        assert!(j.chemin().is_file());
    }
}
