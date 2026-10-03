// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0
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

//! L'interface, en egui.
//!
//! # Le seul crate sans oracle
//!
//! Tous les autres se comparent au Python sur les données réelles de
//! l'utilisateur. Celui-ci ne le peut pas : on ne fige pas des pixels. La
//! parade est de n'y mettre **que** des pixels — chaque règle qui peut se
//! calculer vit ailleurs, dans `ygo-app`, où elle est éprouvée.
//!
//! Deux exceptions notables, et ce sont les seules parties testées ici :
//! [`attenuation`], qui est une transformation pure, et le cache de textures,
//! dont on peut vérifier qu'il ne garde pas deux fois la même image.
//!
//! # Le choix d'egui
//!
//! Tranché le 2026-08-26 sur mesure comparée avec Iced, sur les données
//! réelles de l'utilisateur (cf. §6 du `Construction_Projet`). egui devançait
//! sur le premier rendu (293 ms contre 1 194), la taille du binaire (16,1 Mo
//! contre 27,7), le nombre de dépendances (379 contre 549) et le rendu.

#![forbid(unsafe_code)]

pub mod accueil;
pub mod artworks;
pub mod attenuation;
pub mod classeur;
pub mod corbeille;
pub mod csv;
pub mod fiche;
pub mod images;
pub mod initialisation;
pub mod inventaire;
pub mod options;
pub mod polices;
pub mod selecteur;
pub mod statistiques;
pub mod telechargements;
