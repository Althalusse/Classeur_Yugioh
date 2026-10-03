// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Erreurs de `ygo-core`.
//!
//! Principe repris du Python : **une configuration illisible n'est jamais une
//! erreur bloquante**. Les fonctions de [`crate::config`] retombent sur leur
//! valeur par défaut. `CoreError` ne remonte que pour les opérations où
//! l'appelant peut réellement agir — typiquement l'**écriture** d'un fichier.

use std::path::PathBuf;

/// Alias de résultat du crate.
pub type Result<T> = std::result::Result<T, CoreError>;

/// Erreurs pouvant remonter de `ygo-core`.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// Échec d'une opération sur le système de fichiers, chemin à l'appui.
    #[error("erreur d'entrée/sortie sur {chemin} : {source}")]
    Io {
        /// Chemin concerné.
        chemin: PathBuf,
        /// Erreur d'origine.
        #[source]
        source: std::io::Error,
    },

    /// Sérialisation JSON impossible (écriture de configuration).
    #[error("erreur JSON sur {chemin} : {source}")]
    Json {
        /// Chemin concerné.
        chemin: PathBuf,
        /// Erreur d'origine.
        #[source]
        source: serde_json::Error,
    },

    /// Le dossier racine de l'application n'a pas pu être déterminé.
    #[error("impossible de déterminer le dossier de l'exécutable : {0}")]
    RacineIntrouvable(String),
}

impl CoreError {
    /// Construit une [`CoreError::Io`] en attachant le chemin fautif.
    pub fn io(chemin: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            chemin: chemin.into(),
            source,
        }
    }

    /// Construit une [`CoreError::Json`] en attachant le chemin fautif.
    pub fn json(chemin: impl Into<PathBuf>, source: serde_json::Error) -> Self {
        Self::Json {
            chemin: chemin.into(),
            source,
        }
    }
}
