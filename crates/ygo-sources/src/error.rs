// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Erreurs de `ygo-sources`.
//!
//! Principe repris du Python : **une source indisponible n'est pas une
//! erreur fatale du programme**. `fetch_ygoprodeck()` renvoie `[]` et
//! l'initialisation continue sans les statistiques ; seul YGOJSON est
//! indispensable. La différence est qu'ici l'appelant doit **décider**
//! explicitement, au lieu de recevoir une liste vide indiscernable d'une
//! réponse réellement vide.

use std::path::PathBuf;

/// Alias de résultat du crate.
pub type Result<T> = std::result::Result<T, SourceError>;

/// Erreurs pouvant remonter de `ygo-sources`.
#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    /// Échec réseau ou HTTP.
    #[error("échec réseau sur {url} : {source}")]
    Reseau {
        /// URL concernée.
        url: String,
        /// Erreur d'origine.
        #[source]
        source: reqwest::Error,
    },

    /// Code de statut inattendu.
    #[error("statut HTTP {statut} sur {url}")]
    Statut {
        /// URL concernée.
        url: String,
        /// Code reçu.
        statut: u16,
    },

    /// Réponse illisible (JSON invalide, structure inattendue).
    #[error("réponse illisible depuis {url} : {source}")]
    Deserialisation {
        /// URL ou nom d'entrée d'archive.
        url: String,
        /// Erreur d'origine.
        #[source]
        source: serde_json::Error,
    },

    /// Problème de fichier local — archive temporaire, écriture sur disque.
    #[error("erreur d'entrée/sortie sur {chemin} : {source}")]
    Io {
        /// Chemin concerné.
        chemin: PathBuf,
        /// Erreur d'origine.
        #[source]
        source: std::io::Error,
    },

    /// Archive illisible ou entrée absente.
    #[error("archive illisible : {0}")]
    Archive(String),
}

impl SourceError {
    /// Attache une URL à une erreur `reqwest`.
    pub fn reseau(url: impl Into<String>, source: reqwest::Error) -> Self {
        Self::Reseau {
            url: url.into(),
            source,
        }
    }

    /// Attache un chemin à une erreur d'entrée/sortie.
    pub fn io(chemin: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            chemin: chemin.into(),
            source,
        }
    }

    /// Attache une origine à une erreur de désérialisation.
    pub fn deserialisation(url: impl Into<String>, source: serde_json::Error) -> Self {
        Self::Deserialisation {
            url: url.into(),
            source,
        }
    }
}
