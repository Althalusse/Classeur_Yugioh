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

//! `ygo-sources` — accès aux sources de données externes.
//!
//! Portage de `module/utilitaire/http_client.py`, `module/ygojson_parser.py`,
//! `module/ygoprodeck_enricher.py` et de la partie réseau de
//! `module/version/controle_version_database_api.py`.
//!
//! # Deux corrections structurelles
//!
//! **1. TLS.** Le client Python a été durci en V1.0.4 autour d'un contexte SSL
//! unique partagé, parce que la création concurrente de contextes OpenSSL
//! corrompait l'état natif et produisait une violation d'accès. Ce correctif
//! est ici sans objet : `rustls` est une implémentation TLS en Rust pur, sans
//! état global C. La classe de bug disparaît avec la bibliothèque.
//!
//! **2. Archive YGOJSON.** C'est le correctif le plus important de ce lot.
//! Le Python accumule l'archive en mémoire :
//!
//! ```text
//! chunks = []                          ~150 Mo dans une liste
//! raw_zip = b"".join(chunks)           + une copie contiguë
//! zipfile.ZipFile(io.BytesIO(raw_zip)) + la vue mémoire
//! z.open(...).read()                   + le JSON décompressé
//! json.loads(...)                      + les objets Python
//! ```
//!
//! Cinq représentations simultanées du même jeu de données, sur un fil
//! secondaire. C'est là que se produit la violation d'accès relevée dans
//! `crash_natif.log` le 2026-08-25, à l'intérieur du CRC32 de zlib —
//! **ni la signature A (OpenSSL) ni la B (Tk)** du cahier des charges.
//!
//! Ici, l'archive est **écrite sur disque au fil du téléchargement**, ouverte
//! en place, et chaque entrée JSON est désérialisée en flux. La mémoire est
//! bornée par la taille des données typées, jamais par celle du fichier.
//!
//! # Organisation
//!
//! | Module | Rôle |
//! |---|---|
//! | [`http`] | client unique, quotas du §3.7 |
//! | [`ygojson`] | archive `aggregate.zip` streamée, parsing des cartes et des sets |
//! | [`ygoprodeck`] | statistiques, résolution et expansion des artworks |
//! | [`version`] | `checkDBVer.php` |

#![forbid(unsafe_code)]

pub mod cache;
pub mod error;
pub mod http;
pub mod version;
pub mod ygojson;
pub mod ygoprodeck;
pub mod yugipedia;

pub use error::{Result, SourceError};
pub use http::ClientHttp;
