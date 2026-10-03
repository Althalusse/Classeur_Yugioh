// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Erreurs de `ygo-db`.

use std::path::PathBuf;

/// Alias de résultat du crate.
pub type Result<T> = std::result::Result<T, DbError>;

/// Erreurs pouvant remonter de `ygo-db`.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// Erreur SQLite, avec le chemin de la base concernée.
    #[error("erreur SQLite sur {base} : {source}")]
    Sqlite {
        /// Base concernée.
        base: PathBuf,
        /// Erreur d'origine.
        #[source]
        source: rusqlite::Error,
    },

    /// Erreur SQLite sans base identifiée (connexion en mémoire).
    #[error("erreur SQLite : {0}")]
    SqliteBrute(#[from] rusqlite::Error),

    /// Erreur du système de fichiers.
    #[error("erreur d'entrée/sortie sur {chemin} : {source}")]
    Io {
        /// Chemin concerné.
        chemin: PathBuf,
        /// Erreur d'origine.
        #[source]
        source: std::io::Error,
    },

    /// Le schéma produit diverge du schéma de référence.
    #[error("schéma non conforme : {0} divergence(s)")]
    SchemaNonConforme(usize),

    /// Un verrou de base a été empoisonné par la panique d'un autre fil.
    ///
    /// Ne devrait pas se produire : les crates métier interdisent `panic!`,
    /// `unwrap()` et `expect()` (cf. lints du workspace).
    #[error("verrou de base empoisonné pour {0}")]
    VerrouEmpoisonne(PathBuf),
}

impl DbError {
    /// Attache un chemin de base à une erreur SQLite.
    pub fn sqlite(base: impl Into<PathBuf>, source: rusqlite::Error) -> Self {
        Self::Sqlite {
            base: base.into(),
            source,
        }
    }

    /// Construit une [`DbError::Io`] en attachant le chemin fautif.
    pub fn io(chemin: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            chemin: chemin.into(),
            source,
        }
    }
}
