// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! `ygo-db` — accès SQLite.
//!
//! Portage de `module/db_schema.py`, `module/db_migrations.py` et de la partie
//! « connexions » de `module/centralisation_dossier.py`.
//!
//! # Le schéma n'est pas écrit ici
//!
//! Les DDL vivent dans `assets/schema/*.sql`, **extraits des bases réelles**
//! produites par la V1.0.4 Python (`sqlite3 cardinfo.db .schema`). Ils ne sont
//! jamais retranscrits à la main.
//!
//! C'est une précaution qui a déjà servi : l'Annexe A du cahier des charges
//! décrit `cardinfo.db` comme ayant 8 tables et 9 index, alors que la base
//! réelle en a **11 et 10** — il y manque `anomalies`, `overframe_sync` et
//! `card_images_externes`. Un portage fidèle au document aurait cassé la
//! détection d'anomalies, la passe Overframe et les artworks Yugipedia.

#![forbid(unsafe_code)]

pub mod connexion;
pub mod error;
pub mod init;
pub mod migrations;
pub mod schema;

pub use error::{DbError, Result};

/// Réexport de `rusqlite`.
///
/// Les crates qui manipulent une `Connection` obtenue d'ici doivent employer
/// **exactement** la même version du crate : deux versions de `rusqlite` dans
/// l'arbre donneraient deux types `Connection` incompatibles, avec un message
/// d'erreur qui ne le dit pas. Passer par ce réexport supprime la question.
pub use rusqlite;
