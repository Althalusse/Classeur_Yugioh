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

//! Téléchargement, file d'attente et cache des images de cartes.
//!
//! Portage de `module/img_dl/` — `telechargement_service.py`,
//! `file_attente_classeur.py` — et de la détection de placeholder de
//! `gestion_img/gestion_image_classeur.py`.
//!
//! # Le crash natif que ce module doit ne pas reproduire
//!
//! Le Python écrivait d'abord les images **directement** à leur emplacement
//! final. Le fil de téléchargement écrivait pendant que le fil d'interface
//! relisait le même fichier avec PIL : le lecteur tombait sur un JPEG
//! **tronqué**, libjpeg plantait au niveau C, et l'application disparaissait
//! sans lever la moindre exception Python — donc sans une ligne dans les
//! journaux. C'est l'une des signatures de crash qui ont motivé ce portage.
//!
//! La parade du Python — écrire dans `<nom>.part`, `fsync`, puis `rename` —
//! est reprise telle quelle dans [`ecrire_atomique`]. Rust supprime la classe
//! de bug sous-jacente (pas de décodeur C partagé), mais l'écriture atomique
//! reste nécessaire : un lecteur qui ouvre un fichier à moitié écrit obtient
//! une image tronquée, quel que soit le langage.
//!
//! # Ce que ce crate ne fait pas
//!
//! Il ne lit aucune base. La liste des images d'un classeur se lit dans
//! `ygo-db`, et l'appelant passe le résultat à [`planifier`] — règle R1. Ce
//! crate ne connaît que des URL, des chemins et des octets.

#![forbid(unsafe_code)]

pub mod cache;
pub mod file;
pub mod plan;
pub mod telechargement;

pub use cache::{est_placeholder, SignaturePlaceholder};
pub use file::{Journal, JournalDisque};
pub use plan::{planifier, Bilan, Cible, LigneImage};
pub use telechargement::{ecrire_atomique, Telechargeur, PARALLELISME, TAILLE_MINIMALE};
