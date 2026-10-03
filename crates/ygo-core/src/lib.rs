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

//! `ygo-core` — modèles et règles métier **pures**.
//!
//! Règle d'architecture **R1** du cahier des charges : ce crate ne dépend ni du
//! réseau, ni de SQLite, ni de l'interface. C'est ce qui rend tout le cœur
//! testable hors ligne, donc ce qui rend la migration vérifiable.
//!
//! Modules portés depuis le Python :
//!
//! | Module Rust | Origine Python |
//! |---|---|
//! | [`paths`]   | `module/centralisation_dossier.py` |
//! | [`config`]  | `module/app_config.py` + `module/config/preferences.py` + `module/config_image_source.py` + `module/config_langue.py` |
//! | [`rarity`]  | `module/gestion_rarete/gestion_rarete_service.py` (chargement de `rarity_config.json`) |
//! | [`tri`]     | `module/gestion_rarete/tri_carte.py` |
//! | [`version`] | lecture de `bdd/last_update.txt` |
//! | [`log`]     | `module/logger_app.py` |

#![forbid(unsafe_code)]

pub mod config;
pub mod error;
pub mod image_source;
pub mod log;
pub mod modele;
pub mod paths;
pub mod rarity;
pub mod tri;
pub mod version;

pub use error::{CoreError, Result};
