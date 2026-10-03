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

//! `ygo-app` — orchestration métier.
//!
//! Règle **R6** du cahier des charges : ce crate ne connaît **aucun type
//! d'interface**. Une opération longue expose une **progression par canal**,
//! jamais un rappel.
//!
//! La différence n'est pas cosmétique. En Python, `run_init(log)` reçoit une
//! fonction fournie par la fenêtre d'initialisation et l'appelle depuis un fil
//! secondaire ; c'est ce genre de rappel qui finit par toucher un objet Tk hors
//! du fil de rendu. Ici, [`init::initialiser`] émet des [`init::Etape`] dans un
//! canal `tokio` : l'interface les consomme sur son propre fil, ou personne ne
//! les consomme et rien ne casse.

#![forbid(unsafe_code)]

pub mod accueil;
pub mod adoption;
pub mod anomalies;
pub mod artworks;
pub mod attestation;
pub mod classeur;
pub mod couverture;
pub mod creation;
pub mod doublons;
pub mod error;
pub mod exemplaires;
pub mod export;
pub mod fiche;
pub mod images;
pub mod images_tirage;
pub mod import;
pub mod init;
pub mod inventaire;
pub mod maj;
pub mod noms_fr;
pub mod overframe;
pub mod possession;
pub mod raretes;
pub mod replis;
pub mod scanflip;
pub mod selecteur;
pub mod statistiques;
pub mod suppression;

pub use error::{AppError, Result};
