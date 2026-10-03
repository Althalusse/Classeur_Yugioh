// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! YGOJSON — archive `aggregate.zip`, cartes et sets.
//!
//! Portage de `module/ygojson_parser.py`.
//!
//! # Le correctif du lot : l'archive ne passe plus par la mémoire
//!
//! `crash_natif.log` du 2026-08-25, deux occurrences :
//!
//! ```text
//! Windows fatal exception: access violation
//!   zipfile/__init__.py:1046 in _update_crc
//!   zipfile/__init__.py:1121 in _read1
//!   zipfile/__init__.py:1017 in read
//!   module/ygojson_parser.py:146 in fetch_ygojson
//! ```
//!
//! La violation d'accès est dans le CRC32 de zlib, pendant la lecture d'une
//! archive détenue **entièrement en mémoire** — cinq représentations
//! simultanées du même jeu de données sur un fil secondaire, pendant que le
//! fil principal est bloqué dans `wait_window`.
//!
//! Ici :
//!
//! 1. [`telecharger_archive`] écrit les octets sur disque au fil de l'eau, par
//!    blocs, sans jamais construire de tampon complet ;
//! 2. [`ouvrir_archive`] ouvre le fichier en place ;
//! 3. [`Archive::lire_entree`] désérialise l'entrée JSON **en flux**, directement depuis
//!    le décompresseur.
//!
//! La mémoire est bornée par la taille des données typées, jamais par celle du
//! fichier. C'est l'exigence NF-4 du cahier des charges, et c'est ce qui règle
//! le crash observé.
//!
//! # Parsing
//!
//! [`parse_cartes`] et [`parse_sets`] sont **pures** : elles prennent des
//! structures désérialisées et rendent des lignes de base. Aucun réseau, aucun
//! fichier. Elles se testent donc sur une poignée d'octets de JSON, ce que le
//! Python ne permettait pas.

use std::io::BufReader;
use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use serde::Deserialize;
use tokio::io::AsyncWriteExt;
use ygo_core::modele::{
    CartesParsees, LigneCarte, LigneImage, LigneLocale, LigneSet, LigneTexte, SetsParses,
    TirageBrut,
};

use crate::error::{Result, SourceError};
use crate::http::{ClientHttp, DELAI_TELECHARGEMENT};

/// Archive agrégée du projet YGOJSON.
pub const YGOJSON_URL: &str =
    "https://github.com/iconmaster5326/YGOJSON/releases/download/v1/aggregate.zip";

/// Taille des blocs écrits sur disque pendant le téléchargement.
///
/// Même valeur que le `chunk_size=131072` du Python — il n'y a pas de raison
/// de s'en écarter, et cela garde les journaux de progression comparables.
const TAILLE_BLOC: usize = 131_072;

/// Correspondance des codes de rareté YGOJSON vers les libellés Scanflip.
///
/// Portage à l'identique de `ygojson_parser._RARITY_MAP` — **59 entrées**.
/// Un code absent de cette table fait **ignorer le tirage** : c'est le
/// comportement du Python (`if not rarity: continue`), et il est délibéré.
///
/// Le compte est vérifié par un test : c'est une table recopiée, donc
/// exactement le genre de donnée où une entrée sautée ne se voit pas.
pub const CORRESPONDANCE_RARETES: [(&str, &str); 59] = [
    ("ultra", "Ultra Rare"),
    ("super", "Super Rare"),
    ("rare", "Rare"),
    ("common", "Common"),
    ("collectors", "Collector's Rare"),
    ("secret", "Secret Rare"),
    ("starlight", "Starlight Rare"),
    ("25thsecret", "Quarter Century Secret Rare"),
    ("20thsecret", "20th Secret Rare"),
    ("10000secret", "10000 Secret Rare"),
    ("ultimate", "Ultimate Rare"),
    ("ghost", "Ghost Rare"),
    ("gold", "Gold Rare"),
    ("goldsecret", "Gold Secret Rare"),
    ("goldghost", "Ghost/Gold Parallel Rare"),
    ("premiumgold", "Premium Gold Rare"),
    ("platinum", "Platinum Rare"),
    ("platinumsecret", "Platinum Secret Rare"),
    ("prismaticsecret", "Prismatic Secret Rare"),
    ("grandmaster", "Grand Master Rare"),
    ("grandmasterrare", "Grand Master Rare"),
    ("extrasecret", "Extra Secret Rare"),
    ("extrasecretparallel", "Extra Secret Parallel Rare"),
    ("millenium", "Millennium Rare"),
    ("milleniumultra", "Millennium Ultra Rare"),
    ("milleniumgold", "Millennium Gold Rare"),
    ("milleniumsecret", "Millennium Secret Rare"),
    ("mosaic", "Mosaic Rare"),
    ("shatterfoil", "Shatterfoil Rare"),
    ("starfoil", "Starfoil Rare"),
    ("shortprint", "Short Print"),
    ("commonparallel", "Common Parallel Rare"),
    ("rareparallel", "Rare Parallel Rare"),
    ("superparallel", "Super Parallel Rare"),
    ("ultraparallel", "Ultra Parallel Rare"),
    ("secretparallel", "Secret Parallel Rare"),
    ("ghostparallel", "Ghost/Gold Parallel Rare"),
    ("pharaohs", "Pharaoh's Rare"),
    ("ultrasecret", "Ultra Secret Rare"),
    ("kcrare", "Rare"),
    ("kcultra", "Ultra Rare"),
    ("kccommon", "Common"),
    ("dtpc", "Duel Terminal Common Parallel Rare"),
    ("dtspr", "Duel Terminal Super Parallel Rare"),
    ("dtupr", "Duel Terminal Ultra Parallel Rare"),
    ("dtscpr", "Duel Terminal Secret Parallel Rare"),
    ("dtrpr", "Duel Terminal Rare Parallel Rare"),
    ("dtpsp", "Duel Terminal Normal Parallel Rare"),
    ("rare-blue", "Rare"),
    ("rare-copper", "Rare"),
    ("rare-green", "Rare"),
    ("rare-purple", "Rare"),
    ("rare-red", "Rare"),
    ("rare-wedgewood", "Rare"),
    ("ultra-blue", "Ultra Rare"),
    ("ultra-green", "Ultra Rare"),
    ("ultra-purple", "Ultra Rare"),
    ("secret-blue", "Secret Rare"),
    ("secret-red", "Secret Rare"),
];

/// Traduit un code de rareté YGOJSON. `None` si le code est inconnu.
pub fn rarete_scanflip(code: &str) -> Option<&'static str> {
    CORRESPONDANCE_RARETES
        .iter()
        .find(|(k, _)| *k == code)
        .map(|(_, v)| *v)
}

// ─────────────────────────────────────────────────────────────────────────────
// Structures brutes YGOJSON
// ─────────────────────────────────────────────────────────────────────────────

/// Une carte de `cards.json`.
#[derive(Debug, Clone, Deserialize)]
pub struct CarteBrute {
    /// UUID YGOJSON.
    #[serde(default)]
    pub id: String,
    /// Textes par code langue. L'ordre du JSON est préservé.
    #[serde(default)]
    pub text: IndexMap<String, TexteBrut>,
    /// Illustrations connues.
    #[serde(default)]
    pub images: Vec<ImageBrute>,
    /// Mots de passe Konami, en repli quand aucune image n'en porte.
    #[serde(default)]
    pub passwords: Vec<serde_json::Value>,
    /// `monster`, `spell`, `trap`…
    #[serde(rename = "cardType", default)]
    pub card_type: String,
    /// Sous-catégorie.
    #[serde(default)]
    pub subcategory: String,
}

/// Le bloc de texte d'une carte dans une langue.
#[derive(Debug, Clone, Deserialize)]
pub struct TexteBrut {
    /// Nom de la carte.
    #[serde(default)]
    pub name: Option<String>,
    /// Texte d'effet.
    #[serde(default)]
    pub effect: Option<String>,
    /// Effet pendule, utilisé en repli quand `effect` est vide.
    #[serde(default)]
    pub pendulum_effect: Option<String>,
}

/// Une illustration de carte.
#[derive(Debug, Clone, Deserialize)]
pub struct ImageBrute {
    /// UUID de l'illustration.
    #[serde(default)]
    pub id: String,
    /// Mot de passe Konami — parfois un nombre, parfois une chaîne.
    #[serde(default)]
    pub password: Option<serde_json::Value>,
    /// URL de l'artwork seul.
    #[serde(default)]
    pub art: Option<String>,
    /// URL de la carte entière.
    #[serde(default)]
    pub card: Option<String>,
}

/// Un set de `sets.json`.
#[derive(Debug, Clone, Deserialize)]
pub struct SetBrut {
    /// UUID YGOJSON.
    #[serde(default)]
    pub id: String,
    /// Noms par code langue.
    #[serde(default)]
    pub name: IndexMap<String, String>,
    /// Déclinaisons locales. **L'ordre compte** — il détermine les
    /// `set_locales.id` auto-incrémentés.
    #[serde(default)]
    pub locales: IndexMap<String, LocaleBrute>,
    /// Contenus (un par regroupement d'éditions et de langues).
    #[serde(default)]
    pub contents: Vec<ContenuBrut>,
}

/// Une déclinaison locale d'un set.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LocaleBrute {
    /// Préfixe des codes de set (`RA05-EN`).
    #[serde(default)]
    pub prefix: Option<String>,
    /// Date de sortie.
    #[serde(default)]
    pub date: Option<String>,
    /// URL de la cover du booster.
    #[serde(default)]
    pub image: Option<String>,
    /// `édition → uuid d'illustration → URL`.
    #[serde(rename = "cardImages", default)]
    pub card_images: IndexMap<String, IndexMap<String, serde_json::Value>>,
    /// `édition → uuid d'illustration → objet contenant `image``.
    #[serde(rename = "cardInfo", default)]
    pub card_info: IndexMap<String, IndexMap<String, serde_json::Value>>,
}

/// Un contenu de set.
#[derive(Debug, Clone, Deserialize)]
pub struct ContenuBrut {
    /// Codes langue concernés.
    #[serde(default)]
    pub locales: Vec<String>,
    /// Éditions ; **seule la première est retenue**, comme en Python.
    #[serde(default)]
    pub editions: Vec<String>,
    /// Cartes de ce contenu.
    #[serde(default)]
    pub cards: Vec<CarteContenu>,
}

/// Une carte à l'intérieur d'un contenu de set.
#[derive(Debug, Clone, Deserialize)]
pub struct CarteContenu {
    /// UUID du tirage (distinct de l'UUID d'illustration — cf.
    /// [`crate::ygoprodeck::resoudre_et_etendre_artworks`]).
    #[serde(default)]
    pub id: String,
    /// UUID de la carte.
    #[serde(default)]
    pub card: String,
    /// Suffixe du code de set (`EN134`).
    #[serde(default)]
    pub suffix: String,
    /// Code de rareté YGOJSON, à traduire.
    #[serde(default)]
    pub rarity: String,
    /// Quantité.
    #[serde(default)]
    pub qty: Option<i64>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Téléchargement — streamé sur disque
// ─────────────────────────────────────────────────────────────────────────────

/// Rapport de progression du téléchargement.
pub type Progression<'a> = &'a (dyn Fn(u64, Option<u64>) + Send + Sync);

/// Télécharge `aggregate.zip` **vers un fichier**, sans jamais le charger en
/// mémoire.
///
/// C'est le correctif central de ce lot (NF-4 et signature de crash du
/// 2026-08-25). Retourne le nombre d'octets écrits.
///
/// `progression` est appelée à chaque bloc avec `(octets_reçus, taille_totale)`.
pub async fn telecharger_archive(
    client: &ClientHttp,
    destination: &Path,
    progression: Option<Progression<'_>>,
) -> Result<u64> {
    use futures_util::StreamExt as _;

    let reponse = client
        .get_avec_delai(YGOJSON_URL, DELAI_TELECHARGEMENT)
        .await?;
    let statut = reponse.status();
    if !statut.is_success() {
        return Err(SourceError::Statut {
            url: YGOJSON_URL.to_owned(),
            statut: statut.as_u16(),
        });
    }
    let total = reponse.content_length();

    if let Some(parent) = destination.parent() {
        if !parent.as_os_str().is_empty() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| SourceError::io(parent, e))?;
        }
    }

    let fichier = tokio::fs::File::create(destination)
        .await
        .map_err(|e| SourceError::io(destination, e))?;
    let mut sortie = tokio::io::BufWriter::with_capacity(TAILLE_BLOC, fichier);

    let mut recus: u64 = 0;
    let mut flux = reponse.bytes_stream();
    while let Some(bloc) = flux.next().await {
        let bloc = bloc.map_err(|e| SourceError::reseau(YGOJSON_URL, e))?;
        sortie
            .write_all(&bloc)
            .await
            .map_err(|e| SourceError::io(destination, e))?;
        recus += bloc.len() as u64;
        if let Some(f) = progression {
            f(recus, total);
        }
    }
    sortie
        .flush()
        .await
        .map_err(|e| SourceError::io(destination, e))?;

    tracing::info!(
        octets = recus,
        mo = recus / 1_048_576,
        chemin = %destination.display(),
        "archive YGOJSON téléchargée sur disque"
    );
    Ok(recus)
}

/// Archive YGOJSON ouverte, prête à être lue entrée par entrée.
pub struct Archive {
    zip: zip::ZipArchive<BufReader<std::fs::File>>,
    chemin: PathBuf,
}

impl std::fmt::Debug for Archive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Archive")
            .field("chemin", &self.chemin)
            .field("entrees", &self.zip.len())
            .finish()
    }
}

/// Ouvre l'archive téléchargée.
pub fn ouvrir_archive(chemin: &Path) -> Result<Archive> {
    let fichier = std::fs::File::open(chemin).map_err(|e| SourceError::io(chemin, e))?;
    let zip = zip::ZipArchive::new(BufReader::new(fichier))
        .map_err(|e| SourceError::Archive(format!("{} : {e}", chemin.display())))?;
    Ok(Archive {
        zip,
        chemin: chemin.to_path_buf(),
    })
}

impl Archive {
    /// Noms des entrées de l'archive.
    pub fn entrees(&self) -> Vec<String> {
        self.zip.file_names().map(str::to_owned).collect()
    }

    /// Nom de la première entrée dont le chemin se termine par `suffixe`.
    ///
    /// Reprend le `next((n for n in names if n.endswith("cards.json")), None)`
    /// du Python : l'archive place ces fichiers dans un sous-dossier dont le
    /// nom varie d'une version à l'autre.
    pub fn trouver(&self, suffixe: &str) -> Option<String> {
        self.zip
            .file_names()
            .find(|n| n.ends_with(suffixe))
            .map(str::to_owned)
    }

    /// Désérialise une entrée JSON **en flux**.
    ///
    /// Le décompresseur alimente directement `serde_json` : le texte JSON
    /// n'existe jamais en entier en mémoire, seules les valeurs typées sont
    /// construites.
    pub fn lire_entree<T: serde::de::DeserializeOwned>(&mut self, nom: &str) -> Result<T> {
        let entree = self
            .zip
            .by_name(nom)
            .map_err(|e| SourceError::Archive(format!("entrée `{nom}` : {e}")))?;
        let lecteur = BufReader::with_capacity(TAILLE_BLOC, entree);
        serde_json::from_reader(lecteur).map_err(|e| {
            SourceError::deserialisation(format!("{}#{nom}", self.chemin.display()), e)
        })
    }

    /// Lit `cards.json`. Retourne une liste vide si l'entrée est absente,
    /// comme le Python.
    pub fn cartes(&mut self) -> Result<Vec<CarteBrute>> {
        match self.trouver("cards.json") {
            Some(nom) => self.lire_entree(&nom),
            None => Ok(Vec::new()),
        }
    }

    /// Lit `sets.json`. Retourne une liste vide si l'entrée est absente.
    pub fn sets(&mut self) -> Result<Vec<SetBrut>> {
        match self.trouver("sets.json") {
            Some(nom) => self.lire_entree(&nom),
            None => Ok(Vec::new()),
        }
    }
}

/// Convertit une valeur JSON en entier, en acceptant nombre **et** chaîne.
///
/// Reprend le `int(password)` du Python et son `except (ValueError, TypeError)`.
fn vers_i64(v: &serde_json::Value) -> Option<i64> {
    match v {
        serde_json::Value::Number(n) => n.as_i64(),
        serde_json::Value::String(s) => s.trim().parse::<i64>().ok(),
        _ => None,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Parsing — pur
// ─────────────────────────────────────────────────────────────────────────────

/// Transforme `cards.json` en lignes de base.
///
/// Portage de `parse_ygojson_cards`.
///
/// Le vecteur est **consommé** : chaque carte brute est détruite dès qu'elle a
/// produit ses lignes, au lieu de rester vivante jusqu'à la fin. Sur 14 616
/// cartes portant chacune plusieurs textes et illustrations, la différence de
/// pic mémoire est mesurable.
pub fn parse_cartes(brutes: Vec<CarteBrute>) -> CartesParsees {
    let mut sortie = CartesParsees::default();

    for carte in brutes {
        if carte.id.is_empty() {
            continue;
        }
        let uuid = carte.id;

        // Un nom français présent — même vide de sens — vaut confirmation.
        let a_fr = carte
            .text
            .get("fr")
            .and_then(|t| t.name.as_deref())
            .is_some_and(|n| !n.is_empty());
        if a_fr {
            sortie.uuids_fr_confirmes.insert(uuid.clone());
        }

        for (langue, texte) in carte.text {
            let nom = texte.name.unwrap_or_default();
            if nom.is_empty() {
                continue;
            }
            // `effect` d'abord, `pendulum_effect` en repli — dans cet ordre.
            let effet = texte
                .effect
                .filter(|e| !e.is_empty())
                .or_else(|| texte.pendulum_effect.filter(|e| !e.is_empty()))
                .unwrap_or_default();
            sortie.textes.push(LigneTexte {
                card_uuid: uuid.clone(),
                language: langue,
                name: nom,
                effect: effet,
            });
        }

        let mut premier_password: Option<i64> = None;
        for image in carte.images {
            if image.id.is_empty() {
                continue;
            }
            let password = image.password.as_ref().and_then(vers_i64);
            sortie.images.push(LigneImage {
                uuid: image.id,
                card_uuid: uuid.clone(),
                ygoprodeck_image_id: password,
                art_url: image.art.unwrap_or_default(),
                card_url: image.card.unwrap_or_default(),
            });
            if premier_password.is_none() {
                premier_password = password;
            }
        }

        // Repli sur la liste `passwords` de la carte.
        if premier_password.is_none() {
            premier_password = carte.passwords.iter().find_map(vers_i64);
        }

        sortie.cartes.push(LigneCarte {
            uuid,
            ygoprodeck_id: premier_password,
            card_type: carte.card_type,
            subcategory: carte.subcategory,
            name_fr_confirmed: i64::from(a_fr),
            ..LigneCarte::default()
        });
    }

    sortie
}

/// Transforme `sets.json` en lignes de base.
///
/// Portage de `parse_ygojson_sets`.
///
/// Trois règles à ne pas simplifier :
///
/// - le **nom français d'un set retombe sur l'anglais** quand il manque ;
/// - une rareté absente de [`CORRESPONDANCE_RARETES`] fait **ignorer** le
///   tirage, elle ne produit pas une rareté vide ;
/// - le code de set complet n'est formé que si préfixe **et** suffixe sont
///   présents ; sinon c'est le suffixe seul, ou rien.
pub fn parse_sets(brutes: Vec<SetBrut>) -> SetsParses {
    let mut sortie = SetsParses::default();

    for set in brutes {
        if set.id.is_empty() {
            continue;
        }
        let uuid = set.id;
        let nom = |cle: &str| set.name.get(cle).cloned().unwrap_or_default();
        let nom_en = nom("en");

        sortie.sets.push(LigneSet {
            uuid: uuid.clone(),
            name_fr: {
                let fr = nom("fr");
                if fr.is_empty() {
                    nom_en.clone()
                } else {
                    fr
                }
            },
            name_en: nom_en,
            name_de: nom("de"),
            name_it: nom("it"),
            name_es: nom("es"),
            name_ja: nom("ja"),
            name_ko: nom("ko"),
        });

        for (langue, locale) in &set.locales {
            sortie.locales.push(LigneLocale {
                set_uuid: uuid.clone(),
                language: langue.clone(),
                prefix: locale.prefix.clone().unwrap_or_default(),
                release_date: locale.date.clone().unwrap_or_default(),
                booster_image_url: locale.image.clone().unwrap_or_default(),
            });
        }

        for contenu in &set.contents {
            // Seule la première édition est retenue ; `unlimited` à défaut.
            let edition = contenu
                .editions
                .first()
                .cloned()
                .unwrap_or_else(|| "unlimited".to_owned());

            for entree in &contenu.cards {
                if entree.card.is_empty() || entree.rarity.is_empty() {
                    continue;
                }
                let Some(rarete) = rarete_scanflip(&entree.rarity) else {
                    continue;
                };

                for cle_locale in &contenu.locales {
                    let locale = set.locales.get(cle_locale);
                    let prefixe = locale.and_then(|l| l.prefix.clone()).unwrap_or_default();

                    let code_complet = if !prefixe.is_empty() && !entree.suffix.is_empty() {
                        Some(format!("{prefixe}{}", entree.suffix))
                    } else if !entree.suffix.is_empty() {
                        Some(entree.suffix.clone())
                    } else {
                        None
                    };

                    let url_tirage = locale.and_then(|l| url_de_tirage(l, &edition, &entree.id));

                    sortie.tirages.push(TirageBrut {
                        set_uuid: uuid.clone(),
                        locale_key: cle_locale.clone(),
                        card_uuid: entree.card.clone(),
                        card_image_uuid: (!entree.id.is_empty()).then(|| entree.id.clone()),
                        set_code: code_complet,
                        rarity: rarete.to_owned(),
                        edition: edition.clone(),
                        // `qty` nul ou absent vaut 1 — le `qty if qty else 1`
                        // du Python traite 0 comme absent.
                        qty: entree.qty.filter(|q| *q != 0).unwrap_or(1),
                        print_image_url: url_tirage,
                    });
                }
            }
        }
    }

    sortie
}

/// URL Yugipedia propre à un tirage : `cardImages` d'abord, `cardInfo` ensuite.
fn url_de_tirage(locale: &LocaleBrute, edition: &str, uuid_image: &str) -> Option<String> {
    if uuid_image.is_empty() {
        return None;
    }
    if let Some(serde_json::Value::String(u)) = locale
        .card_images
        .get(edition)
        .and_then(|par_edition| par_edition.get(uuid_image))
    {
        if !u.is_empty() {
            return Some(u.clone());
        }
    }
    locale
        .card_info
        .get(edition)
        .and_then(|par_edition| par_edition.get(uuid_image))
        .and_then(|info| info.get("image"))
        .and_then(serde_json::Value::as_str)
        .filter(|u| !u.is_empty())
        .map(str::to_owned)
}

/// Ouvre une archive et en tire cartes et sets déjà parsés.
///
/// Les deux entrées sont traitées **l'une après l'autre**, et chaque structure
/// brute est libérée dès qu'elle a produit ses lignes. Garder les quatre en vie
/// simultanément — brutes et parsées, cartes et sets — doublerait le pic de
/// mémoire pour rien.
pub fn charger_depuis_archive(chemin: &Path) -> Result<(CartesParsees, SetsParses)> {
    let mut archive = ouvrir_archive(chemin)?;

    let cartes = {
        let brutes = archive.cartes()?;
        tracing::info!(cartes = brutes.len(), "cartes YGOJSON lues");
        parse_cartes(brutes)
    };

    let sets = {
        let bruts = archive.sets()?;
        tracing::info!(sets = bruts.len(), "sets YGOJSON lus");
        parse_sets(bruts)
    };

    Ok((cartes, sets))
}

/// Taille d'un fichier — utilitaire de diagnostic.
pub fn taille_fichier(chemin: &Path) -> Result<u64> {
    std::fs::metadata(chemin)
        .map(|m| m.len())
        .map_err(|e| SourceError::io(chemin, e))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    fn cartes(json: &str) -> CartesParsees {
        parse_cartes(serde_json::from_str(json).unwrap())
    }

    fn sets(json: &str) -> SetsParses {
        parse_sets(serde_json::from_str(json).unwrap())
    }

    // ── Table des raretés ───────────────────────────────────────────────────

    #[test]
    fn la_table_des_raretes_a_59_entrees_sans_doublon() {
        assert_eq!(CORRESPONDANCE_RARETES.len(), 59, "_RARITY_MAP en compte 59");
        let mut codes: Vec<&str> = CORRESPONDANCE_RARETES.iter().map(|(k, _)| *k).collect();
        codes.sort_unstable();
        let avant = codes.len();
        codes.dedup();
        assert_eq!(codes.len(), avant, "un code de rareté apparaît deux fois");
    }

    #[test]
    fn la_correspondance_des_raretes_est_complete() {
        assert_eq!(
            rarete_scanflip("25thsecret"),
            Some("Quarter Century Secret Rare")
        );
        assert_eq!(
            rarete_scanflip("prismaticsecret"),
            Some("Prismatic Secret Rare")
        );
        assert_eq!(rarete_scanflip("grandmaster"), Some("Grand Master Rare"));
        assert_eq!(
            rarete_scanflip("grandmasterrare"),
            Some("Grand Master Rare")
        );
        assert_eq!(
            rarete_scanflip("dtpsp"),
            Some("Duel Terminal Normal Parallel Rare")
        );
        assert_eq!(rarete_scanflip("secret-red"), Some("Secret Rare"));
        // Code inconnu : le tirage sera ignoré, pas doté d'une rareté vide.
        assert_eq!(rarete_scanflip("inconnue"), None);
        assert_eq!(rarete_scanflip(""), None);
    }

    // ── Cartes ──────────────────────────────────────────────────────────────

    #[test]
    fn carte_sans_uuid_est_ignoree() {
        let r = cartes(r#"[{"id": "", "cardType": "monster"}]"#);
        assert!(r.cartes.is_empty());
    }

    #[test]
    fn le_nom_francais_confirme_la_carte() {
        let r = cartes(
            r#"[{
                "id": "u1",
                "cardType": "monster",
                "text": {
                    "en": {"name": "Blue-Eyes White Dragon", "effect": "…"},
                    "fr": {"name": "Dragon Blanc aux Yeux Bleus"}
                }
            }]"#,
        );
        assert_eq!(r.cartes.len(), 1);
        assert_eq!(r.cartes[0].name_fr_confirmed, 1);
        assert!(r.uuids_fr_confirmes.contains("u1"));
        assert_eq!(r.textes.len(), 2);
    }

    #[test]
    fn un_nom_francais_vide_ne_confirme_rien() {
        let r = cartes(r#"[{"id": "u1", "text": {"en": {"name": "X"}, "fr": {"name": ""}}}]"#);
        assert_eq!(r.cartes[0].name_fr_confirmed, 0);
        assert!(r.uuids_fr_confirmes.is_empty());
        // Le texte français vide n'est pas inséré non plus.
        assert_eq!(r.textes.len(), 1);
    }

    #[test]
    fn l_effet_pendule_sert_de_repli() {
        let r = cartes(r#"[{"id": "u1", "text": {"en": {"name": "X", "pendulum_effect": "PE"}}}]"#);
        assert_eq!(r.textes[0].effect, "PE");

        // `effect` a la priorité quand il est renseigné.
        let r = cartes(
            r#"[{"id":"u1","text":{"en":{"name":"X","effect":"E","pendulum_effect":"PE"}}}]"#,
        );
        assert_eq!(r.textes[0].effect, "E");
    }

    #[test]
    fn le_password_vient_de_la_premiere_image_qui_en_a_un() {
        let r = cartes(
            r#"[{
                "id": "u1",
                "images": [
                    {"id": "i1", "art": "a1", "card": "c1"},
                    {"id": "i2", "password": 89631139, "art": "a2", "card": "c2"}
                ]
            }]"#,
        );
        assert_eq!(r.cartes[0].ygoprodeck_id, Some(89631139));
        assert_eq!(r.images.len(), 2);
        assert_eq!(r.images[0].ygoprodeck_image_id, None);
        assert_eq!(r.images[1].ygoprodeck_image_id, Some(89631139));
    }

    #[test]
    fn le_password_accepte_une_chaine() {
        let r = cartes(r#"[{"id":"u1","images":[{"id":"i1","password":"89631139"}]}]"#);
        assert_eq!(r.cartes[0].ygoprodeck_id, Some(89631139));
    }

    #[test]
    fn la_liste_passwords_sert_de_repli() {
        let r = cartes(r#"[{"id":"u1","images":[{"id":"i1"}],"passwords":["x", 12345]}]"#);
        assert_eq!(
            r.cartes[0].ygoprodeck_id,
            Some(12345),
            "une valeur inconvertible est sautée, pas fatale"
        );
    }

    #[test]
    fn une_carte_neuve_n_est_pas_enrichie() {
        let r = cartes(r#"[{"id":"u1","cardType":"monster"}]"#);
        assert!(!r.cartes[0].est_enrichie());
        assert_eq!(r.cartes[0].atk, None);
    }

    // ── Sets ────────────────────────────────────────────────────────────────

    /// Attention à la forme réelle de YGOJSON : le **préfixe** porte le code
    /// langue (`RA05-EN`) et le **suffixe** n'est que le numéro (`134`). Le
    /// code complet est leur concaténation — `RA05-EN` + `134`.
    const SET_RA05: &str = r#"[{
        "id": "s1",
        "name": {"en": "Rarity Collection 5"},
        "locales": {
            "en": {"prefix": "RA05-EN", "date": "2026-01-01", "image": "cover.png"},
            "jp": {"prefix": "RA05-JP"}
        },
        "contents": [{
            "locales": ["en"],
            "editions": ["1st", "unlimited"],
            "cards": [
                {"id": "img1", "card": "c1", "suffix": "134", "rarity": "ultra", "qty": 1},
                {"id": "img2", "card": "c2", "suffix": "141", "rarity": "inconnue"}
            ]
        }]
    }]"#;

    #[test]
    fn le_nom_francais_du_set_retombe_sur_l_anglais() {
        let r = sets(SET_RA05);
        assert_eq!(r.sets[0].name_en, "Rarity Collection 5");
        assert_eq!(r.sets[0].name_fr, "Rarity Collection 5");
        assert_eq!(r.sets[0].name_de, "");
    }

    #[test]
    fn une_rarete_inconnue_fait_ignorer_le_tirage() {
        let r = sets(SET_RA05);
        assert_eq!(r.tirages.len(), 1, "seul le tirage `ultra` survit");
        assert_eq!(r.tirages[0].rarity, "Ultra Rare");
        assert_eq!(r.tirages[0].set_code.as_deref(), Some("RA05-EN134"));
    }

    #[test]
    fn seule_la_premiere_edition_est_retenue() {
        let r = sets(SET_RA05);
        assert_eq!(r.tirages[0].edition, "1st");
    }

    #[test]
    fn l_ordre_des_locales_est_celui_du_json() {
        // Déterminant : `set_locales.id` est AUTOINCREMENT, donc l'ordre
        // d'insertion fixe les identifiants référencés par `set_prints`.
        let r = sets(SET_RA05);
        assert_eq!(r.locales.len(), 2);
        assert_eq!(r.locales[0].language, "en");
        assert_eq!(r.locales[1].language, "jp");
        assert_eq!(r.locales[0].prefix, "RA05-EN");
        assert_eq!(r.locales[0].booster_image_url, "cover.png");
    }

    #[test]
    fn code_de_set_sans_prefixe() {
        let r = sets(
            r#"[{"id":"s1","locales":{"en":{}},
                 "contents":[{"locales":["en"],"cards":[
                    {"id":"i","card":"c","suffix":"EN001","rarity":"common"}]}]}]"#,
        );
        assert_eq!(r.tirages[0].set_code.as_deref(), Some("EN001"));
    }

    #[test]
    fn code_de_set_sans_suffixe_est_nul() {
        let r = sets(
            r#"[{"id":"s1","locales":{"en":{"prefix":"X-EN"}},
                 "contents":[{"locales":["en"],"cards":[
                    {"id":"i","card":"c","suffix":"","rarity":"common"}]}]}]"#,
        );
        assert_eq!(r.tirages[0].set_code, None);
    }

    #[test]
    fn quantite_nulle_ou_absente_vaut_un() {
        let r = sets(
            r#"[{"id":"s1","locales":{"en":{"prefix":"X-"}},
                 "contents":[{"locales":["en"],"cards":[
                    {"id":"i1","card":"c1","suffix":"1","rarity":"common","qty":0},
                    {"id":"i2","card":"c2","suffix":"2","rarity":"common"},
                    {"id":"i3","card":"c3","suffix":"3","rarity":"common","qty":3}]}]}]"#,
        );
        assert_eq!(r.tirages[0].qty, 1);
        assert_eq!(r.tirages[1].qty, 1);
        assert_eq!(r.tirages[2].qty, 3);
    }

    #[test]
    fn url_de_tirage_cardimages_puis_cardinfo() {
        let r = sets(
            r#"[{"id":"s1",
                 "locales":{"en":{"prefix":"X-",
                   "cardImages":{"1st":{"i1":"https://ms.yugipedia.com/a.png"}},
                   "cardInfo":{"1st":{"i2":{"image":"https://ms.yugipedia.com/b.png"}}}}},
                 "contents":[{"locales":["en"],"editions":["1st"],"cards":[
                    {"id":"i1","card":"c1","suffix":"1","rarity":"common"},
                    {"id":"i2","card":"c2","suffix":"2","rarity":"common"},
                    {"id":"i3","card":"c3","suffix":"3","rarity":"common"}]}]}]"#,
        );
        assert_eq!(
            r.tirages[0].print_image_url.as_deref(),
            Some("https://ms.yugipedia.com/a.png")
        );
        assert_eq!(
            r.tirages[1].print_image_url.as_deref(),
            Some("https://ms.yugipedia.com/b.png"),
            "repli sur cardInfo[edition][uuid][\"image\"]"
        );
        assert_eq!(r.tirages[2].print_image_url, None);
    }

    #[test]
    fn un_tirage_est_genere_par_locale_du_contenu() {
        let r = sets(
            r#"[{"id":"s1","locales":{"en":{"prefix":"X-EN"},"fr":{"prefix":"X-FR"}},
                 "contents":[{"locales":["en","fr"],"cards":[
                    {"id":"i","card":"c","suffix":"001","rarity":"common"}]}]}]"#,
        );
        assert_eq!(r.tirages.len(), 2);
        assert_eq!(r.tirages[0].set_code.as_deref(), Some("X-EN001"));
        assert_eq!(r.tirages[1].set_code.as_deref(), Some("X-FR001"));
    }

    // ── Archive ─────────────────────────────────────────────────────────────

    /// Fabrique une archive minimale, comme celle de YGOJSON : les fichiers
    /// sont dans un sous-dossier dont le nom varie.
    fn archive_de_test(dossier: &Path) -> PathBuf {
        use std::io::Write as _;
        use zip::write::SimpleFileOptions;

        let chemin = dossier.join("aggregate.zip");
        let fichier = std::fs::File::create(&chemin).unwrap();
        let mut zip = zip::ZipWriter::new(fichier);
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

        zip.start_file("individual/v1/cards.json", options).unwrap();
        zip.write_all(br#"[{"id":"u1","cardType":"monster","text":{"en":{"name":"X"}}}]"#)
            .unwrap();
        zip.start_file("individual/v1/sets.json", options).unwrap();
        zip.write_all(br#"[{"id":"s1","name":{"en":"S"},"locales":{"en":{"prefix":"S-EN"}}}]"#)
            .unwrap();
        zip.start_file("LISEZMOI.txt", options).unwrap();
        zip.write_all(b"non pertinent").unwrap();
        zip.finish().unwrap();
        chemin
    }

    #[test]
    fn l_archive_se_lit_depuis_le_disque() {
        let tmp = tempfile::tempdir().unwrap();
        let chemin = archive_de_test(tmp.path());

        let mut archive = ouvrir_archive(&chemin).unwrap();
        assert_eq!(archive.entrees().len(), 3);
        assert_eq!(
            archive.trouver("cards.json").as_deref(),
            Some("individual/v1/cards.json"),
            "le sous-dossier varie d'une version à l'autre : on cherche par suffixe"
        );

        let cartes = archive.cartes().unwrap();
        assert_eq!(cartes.len(), 1);
        let sets = archive.sets().unwrap();
        assert_eq!(sets.len(), 1);
    }

    #[test]
    fn une_entree_absente_donne_une_liste_vide() {
        use std::io::Write as _;
        use zip::write::SimpleFileOptions;

        let tmp = tempfile::tempdir().unwrap();
        let chemin = tmp.path().join("vide.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&chemin).unwrap());
        zip.start_file("rien.txt", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"x").unwrap();
        zip.finish().unwrap();

        let mut archive = ouvrir_archive(&chemin).unwrap();
        assert!(archive.cartes().unwrap().is_empty());
        assert!(archive.sets().unwrap().is_empty());
    }

    #[test]
    fn charger_depuis_archive_enchaine_lecture_et_parsing() {
        let tmp = tempfile::tempdir().unwrap();
        let chemin = archive_de_test(tmp.path());

        let (cartes, sets) = charger_depuis_archive(&chemin).unwrap();
        assert_eq!(cartes.cartes.len(), 1);
        assert_eq!(cartes.textes.len(), 1);
        assert_eq!(sets.sets.len(), 1);
        assert_eq!(sets.locales.len(), 1);
    }

    #[test]
    fn une_archive_corrompue_remonte_une_erreur_pas_une_panique() {
        let tmp = tempfile::tempdir().unwrap();
        let chemin = tmp.path().join("corrompue.zip");
        std::fs::write(&chemin, b"ceci n'est pas une archive").unwrap();
        assert!(matches!(
            ouvrir_archive(&chemin),
            Err(SourceError::Archive(_))
        ));
    }
}
