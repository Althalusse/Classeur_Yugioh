// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Erreurs de `ygo-app`.

/// Alias de résultat du crate.
pub type Result<T> = std::result::Result<T, AppError>;

/// Erreurs pouvant remonter de l'orchestration métier.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// Échec d'une source de données.
    #[error(transparent)]
    Source(#[from] ygo_sources::SourceError),

    /// Échec côté base.
    #[error(transparent)]
    Base(#[from] ygo_db::DbError),

    /// Échec côté configuration ou chemins.
    #[error(transparent)]
    Core(#[from] ygo_core::CoreError),

    /// Erreur du système de fichiers.
    #[error("erreur d'entrée/sortie : {0}")]
    Io(#[from] std::io::Error),

    /// Erreur SQLite brute, remontée par une lecture directe.
    #[error("erreur SQLite : {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// Échec de (dé)sérialisation JSON d'une valeur écrite dans `meta`.
    #[error("erreur JSON : {0}")]
    Json(#[from] serde_json::Error),

    /// Le set demandé est absent, ou trop incomplet, dans `cardinfo.db`.
    ///
    /// Portage de `SetNotInLocalDB`. Ce n'est pas une erreur fatale pour
    /// l'appelant : c'est le signal qui fait basculer la création sur
    /// YGOPRODeck.
    #[error("{0}")]
    SetAbsentDeLaBase(String),

    /// Aucune source ne permet de créer le classeur demandé.
    ///
    /// Porte les `ValueError` de `_create_classeur_base`, dont le message est
    /// **destiné à l'utilisateur** : il énumère ce que chaque source a
    /// répondu, parce que « impossible de créer ce classeur » sans dire
    /// pourquoi ne laisse aucune prise pour agir.
    #[error("{0}")]
    Creation(String),

    /// YGOJSON n'a rendu aucune carte — l'initialisation est impossible.
    ///
    /// Reprend le `if not ygojson_cards_raw: return False` du Python.
    /// YGOPRODeck, lui, est facultatif : son absence dégrade sans bloquer.
    #[error("YGOJSON indisponible ou vide — initialisation abandonnée")]
    YgojsonVide,
}
