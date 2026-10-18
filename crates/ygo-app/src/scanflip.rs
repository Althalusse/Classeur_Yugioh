// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Le format CSV de Scanflip — lecture et écriture.
//!
//! Portage de `module/import_csv/import_collection.py` et
//! `module/export/export_collection.py` pour la partie « format ».
//! L'appariement vit dans [`crate::import`], la production des lignes dans
//! [`crate::export`] : ici, il n'y a que la forme du fichier.
//!
//! # Ce que le format a de particulier
//!
//! Onze colonnes, encodage UTF-8 **avec BOM**, fins de ligne CRLF. Trois
//! détails comptent plus que le reste :
//!
//! 1. **L'édition est portée par la colonne, l'état par la valeur.** Les
//!    colonnes `1st Edition`, `Unlimited` et `Limited / Autre` sont
//!    mutuellement exclusives : celle qui est remplie dit l'édition, et ce
//!    qu'elle contient (`NM`, `M`, `EX`…) dit l'état. Sur l'export réel de
//!    l'utilisateur, 401 lignes portent leur état dans `1st Edition` et 6
//!    dans `Unlimited` ; aucune n'en remplit deux.
//!
//! 2. **Le rang d'artwork commence à zéro et s'écrit vide.** La cellule
//!    `N° Artwork` vide vaut « artwork principal », `"1"` le premier
//!    alternatif. Le Python d'avant écrivait vide/2/3 — un décalage d'un
//!    rang que la V1.0.4 a corrigé.
//!
//! 3. **Une ligne par exemplaire physique.** Scanflip n'agrège pas : les
//!    407 lignes de l'export réel portent toutes `Quantité = 1`, et deux
//!    exemplaires identiques donnent deux lignes identiques.
//!
//! # Seules deux colonnes sont obligatoires
//!
//! Leur page d'aide est explicite : `Code` et `Rareté` sont nécessaires,
//! **tout le reste est facultatif**, avec des valeurs par défaut annoncées
//! (`Quantité` vaut 1, `N° Artwork` est vide). Un lecteur qui exigerait les
//! onze colonnes dans l'ordre refuserait un fichier que Scanflip accepte —
//! ce que le nôtre faisait. Les colonnes sont donc repérées **par leur
//! nom**, dans n'importe quel ordre, et celles qui manquent prennent leur
//! valeur par défaut.
//!
//! # Les champs sont cités
//!
//! Vingt et une des 407 lignes réelles contiennent une virgule dans le nom
//! de la carte (« Minerva la Demoiselle, Seigneur Lumière »). C'est la
//! raison du crate `csv` plutôt qu'un `split(',')` : le découpage naïf
//! coupait ces lignes en plein milieu et décalait toutes les colonnes
//! suivantes.
//!
//! # Le format « Application » — 2026-10-18
//!
//! L'export sert aussi à passer une collection d'une installation de
//! l'application à une autre. Les onze colonnes de Scanflip n'y suffisent
//! pas : un code de rareté (`QCR`) recouvre plusieurs libellés, et le rang
//! d'artwork (`N° Artwork`) dépend des lignes de chaque classeur — deux
//! installations dont les passes n'en sont pas au même point ne numérotent
//! pas leurs artworks pareil. L'export « Application » ajoute donc, **après**
//! les onze colonnes de Scanflip, de quoi désigner une ligne sans deviner
//! ([`COLONNES_APPLICATION`]) : la rareté telle qu'en base, le nom anglais,
//! le fichier de l'illustration, son identifiant et le cadre Overframe.
//!
//! Le lecteur les repère par leur nom, comme les autres : un fichier qui les
//! porte rend des lignes dotées d'un [`Complement`], un fichier Scanflip des
//! lignes qui n'en ont pas.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};

/// Les onze colonnes, dans l'ordre exact du format.
pub const COLONNES: [&str; 11] = [
    "Langue",
    "Extension",
    "Code",
    "Nom de la carte",
    "Rareté",
    "1st Edition",
    "Unlimited",
    "Limited / Autre",
    "Quantité",
    "N° Artwork",
    "Reprint",
];

/// Les colonnes que l'export « Application » ajoute après les onze de
/// Scanflip, dans l'ordre où il les écrit.
pub const COLONNES_APPLICATION: [&str; 5] = [
    "Rareté complète",
    "Nom anglais",
    "Illustration",
    "ID image",
    "Overframe",
];

/// Ce qu'une ligne de l'export « Application » dit de plus qu'une ligne
/// Scanflip : de quoi retrouver **sa** ligne de classeur dans une autre
/// installation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Complement {
    /// `Rareté complète` : le libellé de la base (`Quarter Century Secret
    /// Rare`), là où `Rareté` porte le code Scanflip.
    pub rarete: String,
    /// `Nom anglais`, quelle que soit la langue du fichier.
    pub nom_anglais: String,
    /// `Illustration` : le fichier de l'image de la ligne
    /// (`BlueEyesWhiteDragon-RA04-EN-QCScR-1E.png`, `89631139.jpg`).
    pub illustration: String,
    /// `ID image` : `card_image_id`.
    pub id_image: Option<i64>,
    /// `Overframe` : la ligne est un tirage à cadre étendu.
    pub overframe: bool,
}

impl Complement {
    /// Les cinq cellules, dans l'ordre de [`COLONNES_APPLICATION`].
    #[must_use]
    pub fn cellules(&self) -> [String; 5] {
        [
            self.rarete.clone(),
            self.nom_anglais.clone(),
            self.illustration.clone(),
            self.id_image.map(|i| i.to_string()).unwrap_or_default(),
            if self.overframe {
                "1".to_owned()
            } else {
                String::new()
            },
        ]
    }
}

/// Une case « Overframe » : vide, `0`, `non`… valent non ; le reste oui.
fn vrai(texte: &str) -> bool {
    let t = texte.trim().to_lowercase();
    !t.is_empty() && !matches!(t.as_str(), "0" | "non" | "no" | "false" | "faux")
}

/// Les états de conservation, du meilleur au pire.
///
/// Extraits de `QUALITE_TO_CODE` (`export_collection.py`) par lecture de
/// l'AST, dans leur ordre d'apparition : `M` (Mint), `NM` (Near Mint), `EX`
/// (Excellent), `GD` (Good), `PL` (Lightly Played), `PO` (Poor), `DM`
/// (Damaged). C'est l'ordre du Python, et c'est celui d'une échelle — pas
/// l'ordre alphabétique.
pub const ETATS: [&str; 7] = ["M", "NM", "EX", "GD", "PL", "PO", "DM"];

/// L'édition d'un exemplaire — c'est la **colonne** qui la porte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Edition {
    /// `1st Edition`.
    Premiere,
    /// `Unlimited`.
    Illimitee,
    /// `Limited / Autre`.
    Limitee,
}

impl Edition {
    /// Le code stocké en base — celui qu'emploie la V1.0.4.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Premiere => "1st",
            Self::Illimitee => "unlimited",
            Self::Limitee => "limited",
        }
    }

    /// L'en-tête de la colonne qui la porte dans le CSV.
    #[must_use]
    pub fn colonne(self) -> &'static str {
        match self {
            Self::Premiere => "1st Edition",
            Self::Illimitee => "Unlimited",
            Self::Limitee => "Limited / Autre",
        }
    }

    /// Relit le code stocké en base.
    ///
    /// Insensible à la casse : la V1.0.4 écrit `1st` / `unlimited` /
    /// `limited`, mais rien n'empêche une base plus ancienne d'avoir
    /// `Unlimited`.
    #[must_use]
    pub fn depuis_code(code: &str) -> Option<Self> {
        match code.trim().to_ascii_lowercase().as_str() {
            "1st" => Some(Self::Premiere),
            "unlimited" => Some(Self::Illimitee),
            "limited" => Some(Self::Limitee),
            _ => None,
        }
    }

    /// Les trois éditions, dans l'ordre des colonnes.
    #[must_use]
    pub fn toutes() -> [Self; 3] {
        [Self::Premiere, Self::Illimitee, Self::Limitee]
    }
}

/// Une ligne du CSV, décodée.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ligne {
    /// Numéro de la ligne dans le fichier — l'en-tête est la ligne 1.
    ///
    /// Sert au rapport : « ligne 322 » est une adresse que l'utilisateur
    /// peut ouvrir dans son tableur, là où « `RA02-FR001` SPL » ne
    /// distingue pas deux lignes jumelles.
    pub numero: usize,
    /// Libellé de langue, tel qu'écrit par Scanflip.
    pub langue: String,
    /// Code du set — c'est lui qui désigne le classeur.
    pub extension: String,
    /// Code carte, dans la langue de l'exemplaire (`RA02-FR001`).
    pub code: String,
    /// Nom tel que Scanflip l'écrit. **N'entre pas** dans l'appariement.
    pub nom: String,
    /// Code de rareté Scanflip (`C`, `SR`, `QCR`…).
    pub rarete: String,
    /// L'édition, ou `None` si aucune des trois colonnes n'est remplie.
    pub edition: Option<Edition>,
    /// L'état, tel que lu dans la colonne d'édition (`NM`, `M`…).
    pub qualite: String,
    /// Quantité — toujours 1 dans les exports observés, mais le format
    /// autorise davantage.
    pub quantite: i64,
    /// Rang d'artwork : 0 pour le principal (cellule vide), 1 pour le
    /// premier alternatif.
    pub artwork: u32,
    /// Colonne `Reprint`, conservée telle quelle pour l'aller-retour.
    pub reprint: String,
    /// Les colonnes de l'export « Application », si le fichier les porte.
    pub complement: Option<Complement>,
}

/// Ce qui a empêché de décoder une ligne.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LigneIllisible {
    /// Numéro de la ligne dans le fichier.
    pub numero: usize,
    /// Ce qui cloche, en clair.
    pub raison: String,
}

/// Le résultat de la lecture d'un fichier.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Lecture {
    /// Les lignes décodées.
    pub lignes: Vec<Ligne>,
    /// Celles qui ne l'ont pas été, avec leur raison.
    pub illisibles: Vec<LigneIllisible>,
    /// Le fichier porte les colonnes de l'export « Application ».
    pub application: bool,
}

/// Décode la quantité.
///
/// Une quantité vide, négative ou non numérique n'est pas une quantité :
/// la ligne est écartée plutôt qu'importée à zéro, parce qu'importer zéro
/// **effacerait** une possession que le CSV n'a jamais voulu nier.
fn quantite(texte: &str) -> Option<i64> {
    let t = texte.trim();
    if t.is_empty() {
        return None;
    }
    t.parse::<i64>().ok().filter(|n| *n >= 0)
}

/// Décode la cellule `N° Artwork`.
///
/// Vide vaut 0 — c'est la convention Scanflip, et c'est le cas de 391 des
/// 407 lignes réelles. Une valeur illisible vaut 0 elle aussi : le rang
/// principal est le repli le moins surprenant, et le Python fait pareil.
#[must_use]
pub fn artwork(texte: &str) -> u32 {
    let t = texte.trim();
    if t.is_empty() {
        return 0;
    }
    t.parse::<u32>().unwrap_or(0)
}

/// Écrit un rang d'artwork : le rang 0 s'écrit **vide**.
#[must_use]
pub fn artwork_ecrit(rang: u32) -> String {
    if rang == 0 {
        String::new()
    } else {
        rang.to_string()
    }
}

/// Lit la colonne d'édition remplie, et l'état qu'elle porte.
///
/// Si plusieurs sont remplies — CSV malformé, jamais observé —, la
/// première dans l'ordre des colonnes gagne, comme dans le Python.
fn edition_et_qualite(champs: &[String; 3]) -> (Option<Edition>, String) {
    for (i, edition) in Edition::toutes().into_iter().enumerate() {
        let valeur = champs.get(i).map_or("", |s| s.trim());
        if !valeur.is_empty() {
            return (Some(edition), valeur.to_owned());
        }
    }
    (None, String::new())
}

/// Lit un fichier au format Scanflip.
///
/// Le BOM est retiré par `csv` lui-même dès lors que l'en-tête est lu comme
/// tel ; les fins de ligne CRLF et LF sont acceptées indifféremment.
///
/// # Errors
///
/// Rend une erreur si le fichier est illisible ou si son en-tête n'est pas
/// celui de Scanflip — mieux vaut refuser un fichier entier que d'importer
/// des colonnes décalées dans la collection de quelqu'un.
pub fn lire(chemin: &Path) -> Result<Lecture> {
    let octets = std::fs::read(chemin)
        .map_err(|e| AppError::Creation(format!("{} : {e}", chemin.display())))?;
    lire_octets(&octets)
}

/// Où se trouve chaque colonne dans un fichier donné.
///
/// Chaque champ porte l'indice de sa colonne, ou `None` si le fichier ne
/// l'a pas. Seuls `code` et `rarete` sont exigés ; les autres se replient
/// sur les valeurs par défaut publiées par Scanflip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Colonnes {
    /// `Code` — obligatoire.
    pub code: usize,
    /// `Rareté` — obligatoire.
    pub rarete: usize,
    /// `Langue`.
    pub langue: Option<usize>,
    /// `Extension` — déduite du code quand elle manque.
    pub extension: Option<usize>,
    /// `Nom de la carte`.
    pub nom: Option<usize>,
    /// Les trois colonnes d'édition, dans l'ordre de [`Edition::toutes`].
    pub editions: [Option<usize>; 3],
    /// `Quantité` — vaut 1 quand elle manque.
    pub quantite: Option<usize>,
    /// `N° Artwork` — vaut 0 quand elle manque.
    pub artwork: Option<usize>,
    /// `Reprint`.
    pub reprint: Option<usize>,
    /// Les colonnes de l'export « Application », dans l'ordre de
    /// [`COLONNES_APPLICATION`].
    pub application: [Option<usize>; 5],
}

impl Colonnes {
    /// Le fichier porte-t-il au moins une colonne de l'export
    /// « Application » ?
    #[must_use]
    pub fn a_le_complement(&self) -> bool {
        self.application.iter().any(Option::is_some)
    }
}

/// Repère les colonnes d'un en-tête.
///
/// La comparaison est faite sur une forme normalisée — sans accents, sans
/// espaces, sans casse — parce qu'un fichier retouché à la main écrit
/// volontiers `Rarete` ou `quantite`, et que refuser pour un accent serait
/// mesquin.
///
/// # Errors
///
/// Rend une erreur si `Code` ou `Rareté` manque : sans l'un des deux, une
/// ligne ne désigne aucune carte, et importer devient deviner.
pub fn reperer(entete: &[String]) -> Result<Colonnes> {
    let normalise: Vec<String> = entete
        .iter()
        .map(|c| ygo_core::rarity::reference::normaliser(c.trim()))
        .collect();
    let cherche = |nom: &str| {
        let vise = ygo_core::rarity::reference::normaliser(nom);
        normalise.iter().position(|c| *c == vise)
    };
    let manque = |nom: &str| {
        AppError::Creation(format!(
            "colonne « {nom} » absente. Scanflip exige « Code » et « Rareté » ;              les autres colonnes sont facultatives.\n  lu : {}",
            entete.join(", ")
        ))
    };
    Ok(Colonnes {
        code: cherche("Code").ok_or_else(|| manque("Code"))?,
        rarete: cherche("Rareté").ok_or_else(|| manque("Rareté"))?,
        langue: cherche("Langue"),
        extension: cherche("Extension"),
        nom: cherche("Nom de la carte"),
        editions: [
            cherche("1st Edition"),
            cherche("Unlimited"),
            cherche("Limited / Autre"),
        ],
        quantite: cherche("Quantité"),
        artwork: cherche("N° Artwork"),
        reprint: cherche("Reprint"),
        application: COLONNES_APPLICATION.map(cherche),
    })
}

/// Le classeur qu'un code carte désigne, quand la colonne `Extension`
/// manque.
///
/// Le classeur est le préfixe du code — sauf pour l'**OCG**, dont le code
/// de langue fait partie du nom du classeur : `LOCR-JP001` appartient au
/// classeur `LOCR-JP`, quand `SDLI-FR001` appartient à `SDLI`.
///
/// C'est une déduction, pas une lecture : un vrai export Scanflip porte
/// toujours sa colonne `Extension`, et elle fait foi. Celle-ci ne sert que
/// pour un fichier réduit aux deux colonnes obligatoires.
///
/// ```
/// use ygo_app::scanflip::extension_deduite;
/// assert_eq!(extension_deduite("SDLI-FR001"), "SDLI");
/// assert_eq!(extension_deduite("LDK2-FRJ01"), "LDK2");
/// assert_eq!(extension_deduite("LOCR-JP001"), "LOCR-JP");
/// assert_eq!(extension_deduite("PROMO"), "PROMO");
/// ```
#[must_use]
pub fn extension_deduite(code: &str) -> String {
    let code = code.trim();
    let Some(tiret) = code.rfind('-') else {
        return code.to_owned();
    };
    let (prefixe, reste) = code.split_at(tiret);
    let langue: String = reste
        .trim_start_matches('-')
        .chars()
        .take(2)
        .flat_map(char::to_uppercase)
        .collect();
    if ygo_core::config::SUFFIXES_OCG.contains(&langue.as_str()) {
        format!("{prefixe}-{langue}")
    } else {
        prefixe.to_owned()
    }
}

/// Lit un CSV déjà en mémoire — c'est la fonction que les tests emploient.
///
/// # Errors
///
/// Voir [`lire`].
pub fn lire_octets(octets: &[u8]) -> Result<Lecture> {
    // `csv` ne retire pas le BOM tout seul : il se retrouverait collé au
    // premier en-tête, et « ﻿Langue » ne serait pas « Langue ».
    let sans_bom = octets.strip_prefix(b"\xef\xbb\xbf").unwrap_or(octets);
    let mut lecteur = csv::ReaderBuilder::new()
        .flexible(true)
        .from_reader(sans_bom);

    let entete: Vec<String> = lecteur
        .headers()
        .map_err(|e| AppError::Creation(format!("en-tête illisible : {e}")))?
        .iter()
        .map(|s| s.trim().to_owned())
        .collect();
    let colonnes = reperer(&entete)?;

    let mut lecture = Lecture {
        application: colonnes.a_le_complement(),
        ..Lecture::default()
    };
    for (i, resultat) in lecteur.records().enumerate() {
        let numero = i + 2; // la ligne 1 est l'en-tête
        let enregistrement = match resultat {
            Ok(r) => r,
            Err(e) => {
                lecture.illisibles.push(LigneIllisible {
                    numero,
                    raison: e.to_string(),
                });
                continue;
            }
        };
        let champ = |n: Option<usize>| {
            n.and_then(|i| enregistrement.get(i))
                .unwrap_or("")
                .trim()
                .to_owned()
        };
        let obligatoire = |n: usize| enregistrement.get(n).unwrap_or("").trim().to_owned();

        let editions = colonnes.editions.map(champ);
        let (edition, qualite) = edition_et_qualite(&editions);

        // Colonne absente : Scanflip annonce 1 par défaut. Colonne
        // présente mais illisible : la ligne est écartée, parce qu'une
        // quantité qu'on ne sait pas lire n'est pas une quantité.
        let quantite = match colonnes.quantite {
            None => 1,
            Some(i) => {
                let brut = enregistrement.get(i).unwrap_or("").trim().to_owned();
                match quantite(&brut) {
                    Some(q) => q,
                    None => {
                        lecture.illisibles.push(LigneIllisible {
                            numero,
                            raison: format!("quantité illisible : {brut:?}"),
                        });
                        continue;
                    }
                }
            }
        };

        let code = obligatoire(colonnes.code);
        let extension = match colonnes.extension {
            Some(i) if !enregistrement.get(i).unwrap_or("").trim().is_empty() => champ(Some(i)),
            _ => extension_deduite(&code),
        };
        let complement = colonnes.a_le_complement().then(|| {
            let [rarete, nom_anglais, illustration, id_image, overframe] =
                colonnes.application.map(champ);
            Complement {
                rarete,
                nom_anglais,
                illustration,
                // Un identifiant illisible n'identifie rien : il compte comme
                // absent, la ligne reste importable par son rang.
                id_image: id_image.parse::<i64>().ok(),
                overframe: vrai(&overframe),
            }
        });

        lecture.lignes.push(Ligne {
            numero,
            langue: champ(colonnes.langue),
            extension,
            code,
            nom: champ(colonnes.nom),
            rarete: obligatoire(colonnes.rarete),
            edition,
            qualite,
            quantite,
            artwork: artwork(&champ(colonnes.artwork)),
            reprint: champ(colonnes.reprint),
            complement,
        });
    }
    Ok(lecture)
}

/// Une ligne prête à écrire.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sortie {
    /// Libellé de langue.
    pub langue: String,
    /// Code du set.
    pub extension: String,
    /// Code carte.
    pub code: String,
    /// Nom de la carte.
    pub nom: String,
    /// Code de rareté Scanflip.
    pub rarete: String,
    /// Édition — `None` écrit l'état dans `1st Edition`, comme le Python.
    pub edition: Option<Edition>,
    /// État.
    pub qualite: String,
    /// Quantité.
    pub quantite: i64,
    /// Rang d'artwork.
    pub artwork: u32,
    /// Les colonnes de l'export « Application » ; `None` pour une ligne
    /// Scanflip.
    pub complement: Option<Complement>,
}

impl Sortie {
    /// Les onze cellules, dans l'ordre des colonnes.
    #[must_use]
    pub fn cellules(&self) -> [String; 11] {
        // Une édition absente écrit dans « 1st Edition » : c'est le repli du
        // Python (`DEFAULT_EDITION_COLUMN`), et il concerne toutes les
        // lignes saisies dans l'application avant que l'import ne renseigne
        // l'édition.
        let colonne = self.edition.unwrap_or(Edition::Premiere).colonne();
        let cellule = |c: &str| {
            if c == colonne {
                self.qualite.clone()
            } else {
                String::new()
            }
        };
        [
            self.langue.clone(),
            self.extension.clone(),
            self.code.clone(),
            self.nom.clone(),
            self.rarete.clone(),
            cellule("1st Edition"),
            cellule("Unlimited"),
            cellule("Limited / Autre"),
            self.quantite.to_string(),
            artwork_ecrit(self.artwork),
            String::new(),
        ]
    }
}

/// Écrit un fichier au format Scanflip : BOM, CRLF, onze colonnes.
///
/// # Errors
///
/// Rend une erreur si le fichier ne peut pas être écrit.
pub fn ecrire(chemin: &Path, lignes: &[Sortie]) -> Result<()> {
    let octets = ecrire_octets(lignes)?;
    if let Some(parent) = chemin.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| AppError::Creation(format!("{} : {e}", parent.display())))?;
    }
    std::fs::write(chemin, octets)
        .map_err(|e| AppError::Creation(format!("{} : {e}", chemin.display())))
}

/// Produit les octets du fichier — BOM compris.
///
/// Les colonnes de l'export « Application » ne s'écrivent que si une ligne
/// au moins porte un [`Complement`] : un export Scanflip garde ses onze
/// colonnes, au caractère près.
///
/// # Errors
///
/// Rend une erreur si la sérialisation échoue, ce qui ne peut arriver que
/// sur une écriture mémoire impossible.
pub fn ecrire_octets(lignes: &[Sortie]) -> Result<Vec<u8>> {
    let application = lignes.iter().any(|l| l.complement.is_some());
    let mut sortie = csv::WriterBuilder::new()
        .terminator(csv::Terminator::CRLF)
        .from_writer(Vec::new());
    let mut entete: Vec<&str> = COLONNES.to_vec();
    if application {
        entete.extend(COLONNES_APPLICATION);
    }
    sortie
        .write_record(&entete)
        .map_err(|e| AppError::Creation(e.to_string()))?;
    for ligne in lignes {
        let mut cellules: Vec<String> = ligne.cellules().to_vec();
        if application {
            cellules.extend(ligne.complement.clone().unwrap_or_default().cellules());
        }
        sortie
            .write_record(&cellules)
            .map_err(|e| AppError::Creation(e.to_string()))?;
    }
    let corps = sortie
        .into_inner()
        .map_err(|e| AppError::Creation(e.to_string()))?;
    let mut octets = Vec::with_capacity(corps.len() + 3);
    octets.extend_from_slice(b"\xef\xbb\xbf");
    octets.extend_from_slice(&corps);
    Ok(octets)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    /// L'export réel de l'utilisateur, tel qu'il sort de Scanflip.
    const REEL: &[u8] = include_bytes!("../../../tests/fixtures/oracle/csv/scanflip_20260829.csv");

    #[test]
    fn le_fichier_reel_se_lit_en_entier() {
        let lecture = lire_octets(REEL).unwrap();
        assert_eq!(lecture.lignes.len(), 407);
        assert!(lecture.illisibles.is_empty(), "{:?}", lecture.illisibles);
        assert_eq!(lecture.lignes[0].numero, 2, "l'en-tête est la ligne 1");
    }

    /// Les noms cités contiennent des virgules. C'est la raison d'être du
    /// vrai lecteur CSV : un `split(',')` décalerait toutes les colonnes
    /// suivantes, et la rareté se lirait dans la moitié d'un nom.
    #[test]
    fn un_nom_cite_ne_decale_pas_les_colonnes() {
        let lecture = lire_octets(REEL).unwrap();
        let l = lecture
            .lignes
            .iter()
            .find(|l| l.code == "SDLI-FR002")
            .unwrap();
        assert_eq!(l.nom, "Minerva la Demoiselle, Seigneur Lumière");
        assert_eq!(l.rarete, "SR", "la rareté est bien la colonne suivante");
        assert_eq!(l.quantite, 1);

        let cites = lecture
            .lignes
            .iter()
            .filter(|l| l.nom.contains(','))
            .count();
        assert!(cites >= 20, "{cites} noms contiennent une virgule");
    }

    /// L'édition est portée par la colonne, l'état par la valeur.
    #[test]
    fn l_edition_se_lit_dans_la_colonne_et_l_etat_dans_la_valeur() {
        let lecture = lire_octets(REEL).unwrap();
        let premiere = lecture
            .lignes
            .iter()
            .filter(|l| l.edition == Some(Edition::Premiere))
            .count();
        let illimitee = lecture
            .lignes
            .iter()
            .filter(|l| l.edition == Some(Edition::Illimitee))
            .count();
        assert_eq!(
            (premiere, illimitee),
            (401, 6),
            "compté sur le fichier réel"
        );
        assert!(
            lecture.lignes.iter().all(|l| l.edition.is_some()),
            "aucune ligne réelle n'est sans édition"
        );

        // VASM-FR001 est en Unlimited, état M — le seul de son espèce.
        let l = lecture
            .lignes
            .iter()
            .find(|l| l.code == "VASM-FR001")
            .unwrap();
        assert_eq!(l.edition, Some(Edition::Illimitee));
        assert_eq!(l.qualite, "M");
    }

    /// Le rang d'artwork : vide vaut 0, `"1"` vaut 1.
    #[test]
    fn le_rang_d_artwork_part_de_zero() {
        assert_eq!(artwork(""), 0);
        assert_eq!(artwork("  "), 0);
        assert_eq!(artwork("1"), 1);
        assert_eq!(artwork("2"), 2);
        assert_eq!(artwork("bof"), 0, "illisible → principal");
        assert_eq!(artwork_ecrit(0), "", "le rang 0 s'écrit vide");
        assert_eq!(artwork_ecrit(1), "1");

        let lecture = lire_octets(REEL).unwrap();
        let alternatifs = lecture.lignes.iter().filter(|l| l.artwork > 0).count();
        assert_eq!(alternatifs, 16, "compté sur le fichier réel");
    }

    /// Une quantité qui n'en est pas écarte la ligne — elle ne l'importe
    /// pas à zéro, ce qui **effacerait** une possession.
    #[test]
    fn une_quantite_illisible_ecarte_la_ligne() {
        assert_eq!(quantite("1"), Some(1));
        assert_eq!(quantite(" 3 "), Some(3));
        assert_eq!(quantite("0"), Some(0));
        assert_eq!(quantite(""), None);
        assert_eq!(quantite("deux"), None);
        assert_eq!(quantite("-1"), None, "une quantité négative n'existe pas");

        let mut csv = String::from(&COLONNES.join(","));
        csv.push_str("\nFrançais (France),RA02,RA02-FR001,Chat,C,NM,,,deux,,\n");
        let lecture = lire_octets(csv.as_bytes()).unwrap();
        assert!(lecture.lignes.is_empty());
        assert_eq!(lecture.illisibles.len(), 1);
        assert_eq!(lecture.illisibles[0].numero, 2);
    }

    /// Un fichier sans `Code` ni `Rareté` est refusé, et l'erreur dit
    /// laquelle manque.
    #[test]
    fn les_deux_colonnes_obligatoires_sont_exigees() {
        let erreur = lire_octets(b"Nom,Quantite\nA,1\n").unwrap_err().to_string();
        assert!(erreur.contains("« Code » absente"), "{erreur}");
        assert!(
            erreur.contains("Nom, Quantite"),
            "il montre ce qu'il a lu : {erreur}"
        );

        let erreur = lire_octets(b"Code\nSDLI-FR001\n").unwrap_err().to_string();
        assert!(erreur.contains("« Rareté » absente"), "{erreur}");
    }

    /// Le fichier minimal que les règles de Scanflip autorisent.
    ///
    /// « Colonnes nécessaires : Code, Rareté. » Tout le reste est
    /// facultatif, avec des valeurs par défaut annoncées. Notre lecteur
    /// exigeait les onze colonnes dans l'ordre et refusait ce fichier —
    /// alors que Scanflip l'accepte.
    #[test]
    fn deux_colonnes_suffisent() {
        let lecture = lire_octets(b"Code,Rarete\nSDLI-FR001,C\nLOCR-JP001,PRI\n").unwrap();
        assert!(lecture.illisibles.is_empty());
        assert_eq!(lecture.lignes.len(), 2);

        let l = &lecture.lignes[0];
        assert_eq!(l.code, "SDLI-FR001");
        assert_eq!(l.rarete, "C");
        assert_eq!(l.quantite, 1, "la valeur par défaut annoncée");
        assert_eq!(l.artwork, 0);
        assert_eq!(l.edition, None);
        assert_eq!(l.qualite, "");
        assert_eq!(l.extension, "SDLI", "déduite du code");
        assert_eq!(
            lecture.lignes[1].extension, "LOCR-JP",
            "l'OCG garde sa langue"
        );
    }

    /// Les colonnes sont repérées par leur nom, dans n'importe quel ordre.
    #[test]
    fn l_ordre_des_colonnes_est_indifferent() {
        let lecture = lire_octets("Rareté,Quantité,Code\nSCR,3,RA02-FR001\n".as_bytes()).unwrap();
        let l = &lecture.lignes[0];
        assert_eq!(l.code, "RA02-FR001");
        assert_eq!(l.rarete, "SCR");
        assert_eq!(l.quantite, 3);
    }

    /// Un en-tête sans accent ni casse exacte est accepté.
    #[test]
    fn l_en_tete_tolere_les_accents_et_la_casse() {
        for entete in [
            "Code,Rareté,Quantité",
            "Code,Rarete,Quantite",
            "CODE,RARETE,QUANTITE",
            "code, rareté , quantité",
        ] {
            let csv = format!("{entete}\nSDLI-FR001,C,2\n");
            let lecture = lire_octets(csv.as_bytes()).unwrap();
            assert_eq!(lecture.lignes[0].quantite, 2, "« {entete} »");
        }
    }

    /// La colonne `Extension` fait foi quand elle est là — la déduction
    /// n'est qu'un repli.
    #[test]
    fn la_colonne_extension_prime_sur_la_deduction() {
        let avec =
            lire_octets("Extension,Code,Rareté\nMonClasseur,SDLI-FR001,C\n".as_bytes()).unwrap();
        assert_eq!(avec.lignes[0].extension, "MonClasseur");
        let vide = lire_octets("Extension,Code,Rareté\n,SDLI-FR001,C\n".as_bytes()).unwrap();
        assert_eq!(vide.lignes[0].extension, "SDLI", "vide, on déduit");
    }

    /// La déduction du classeur : le préfixe, sauf pour l'OCG.
    #[test]
    fn le_classeur_se_deduit_du_code_sauf_pour_l_ocg() {
        for (code, attendu) in [
            ("SDLI-FR001", "SDLI"),
            ("RA02-EN001", "RA02"),
            ("LDK2-FRJ01", "LDK2"),
            ("VASM-FR060", "VASM"),
        ] {
            assert_eq!(extension_deduite(code), attendu, "{code}");
        }
        for (code, attendu) in [
            ("LOCR-JP001", "LOCR-JP"),
            ("LOCH-JP012", "LOCH-JP"),
            ("CROS-KR001", "CROS-KR"),
        ] {
            assert_eq!(extension_deduite(code), attendu, "{code} garde sa langue");
        }
        assert_eq!(extension_deduite("PROMO"), "PROMO");
        assert_eq!(extension_deduite(""), "");
    }

    /// Le BOM ne doit pas se retrouver collé au premier en-tête.
    #[test]
    fn le_bom_est_retire() {
        assert_eq!(&REEL[..3], b"\xef\xbb\xbf", "le fichier réel en a un");
        let sans = lire_octets(&REEL[3..]).unwrap();
        let avec = lire_octets(REEL).unwrap();
        assert_eq!(sans.lignes.len(), avec.lignes.len());
    }

    /// L'aller-retour : ce qu'on écrit se relit à l'identique.
    #[test]
    fn ce_qui_est_ecrit_se_relit() {
        let sorties = vec![
            Sortie {
                langue: "Français (France)".into(),
                extension: "SDLI".into(),
                code: "SDLI-FR002".into(),
                nom: "Minerva la Demoiselle, Seigneur Lumière".into(),
                rarete: "SR".into(),
                edition: Some(Edition::Premiere),
                qualite: "NM".into(),
                quantite: 1,
                artwork: 0,
                complement: None,
            },
            Sortie {
                langue: "Anglais (Monde)".into(),
                extension: "RA02".into(),
                code: "RA02-EN001".into(),
                nom: "Rescue \"Cat\"".into(),
                rarete: "QCR".into(),
                edition: Some(Edition::Illimitee),
                qualite: "M".into(),
                quantite: 3,
                artwork: 1,
                complement: None,
            },
        ];
        let octets = ecrire_octets(&sorties).unwrap();
        assert_eq!(&octets[..3], b"\xef\xbb\xbf", "le BOM est écrit");
        assert!(
            octets.windows(2).any(|f| f == b"\r\n"),
            "les fins de ligne sont en CRLF"
        );

        let relu = lire_octets(&octets).unwrap();
        assert!(relu.illisibles.is_empty());
        assert_eq!(relu.lignes.len(), 2);
        assert_eq!(
            relu.lignes[0].nom,
            "Minerva la Demoiselle, Seigneur Lumière"
        );
        assert_eq!(relu.lignes[1].nom, "Rescue \"Cat\"", "les guillemets aussi");
        assert_eq!(relu.lignes[1].artwork, 1);
        assert_eq!(relu.lignes[1].edition, Some(Edition::Illimitee));
        assert_eq!(relu.lignes[1].qualite, "M");
        assert_eq!(relu.lignes[1].quantite, 3);
    }

    /// Une seule colonne d'édition est remplie à l'écriture.
    #[test]
    fn une_seule_colonne_d_edition_est_remplie() {
        for edition in Edition::toutes() {
            let s = Sortie {
                langue: String::new(),
                extension: String::new(),
                code: String::new(),
                nom: String::new(),
                rarete: String::new(),
                edition: Some(edition),
                qualite: "NM".into(),
                quantite: 1,
                artwork: 0,
                complement: None,
            };
            let cellules = s.cellules();
            let remplies: Vec<&String> = cellules[5..8].iter().filter(|c| !c.is_empty()).collect();
            assert_eq!(remplies.len(), 1, "{edition:?}");
            assert_eq!(remplies[0], "NM");
            let attendue = match edition {
                Edition::Premiere => 5,
                Edition::Illimitee => 6,
                Edition::Limitee => 7,
            };
            assert_eq!(
                cellules[attendue], "NM",
                "{edition:?} écrit dans sa colonne"
            );
        }
    }

    /// Une édition absente écrit dans « 1st Edition » — le repli du Python.
    #[test]
    fn une_edition_absente_se_replie_sur_la_premiere() {
        let s = Sortie {
            langue: String::new(),
            extension: String::new(),
            code: String::new(),
            nom: String::new(),
            rarete: String::new(),
            edition: None,
            qualite: "EX".into(),
            quantite: 1,
            artwork: 0,
            complement: None,
        };
        assert_eq!(s.cellules()[5], "EX");
        assert_eq!(s.cellules()[6], "");
    }

    /// Les états sont ceux du Python, dans l'ordre de l'échelle.
    #[test]
    fn les_etats_vont_du_meilleur_au_pire() {
        assert_eq!(ETATS.first(), Some(&"M"), "Mint en tête");
        assert_eq!(ETATS.last(), Some(&"DM"), "Damaged en queue");
        assert_eq!(ETATS.len(), 7);
        let uniques: std::collections::HashSet<&&str> = ETATS.iter().collect();
        assert_eq!(uniques.len(), ETATS.len(), "aucun doublon");
        // Ce n'est PAS l'ordre alphabétique : c'est une échelle.
        let mut trie = ETATS;
        trie.sort_unstable();
        assert_ne!(trie, ETATS);
    }

    /// Les codes d'édition font l'aller-retour avec la base.
    #[test]
    fn les_codes_d_edition_font_l_aller_retour() {
        for e in Edition::toutes() {
            assert_eq!(Edition::depuis_code(e.code()), Some(e));
        }
        assert_eq!(Edition::depuis_code("Unlimited"), Some(Edition::Illimitee));
        assert_eq!(Edition::depuis_code(""), None);
        assert_eq!(Edition::depuis_code("premiere"), None);
    }

    fn sortie_application(complement: Option<Complement>) -> Sortie {
        Sortie {
            langue: "Français (France)".into(),
            extension: "RA04".into(),
            code: "RA04-EN002".into(),
            nom: "Dragon Blanc aux Yeux Bleus".into(),
            rarete: "QCR".into(),
            edition: Some(Edition::Premiere),
            qualite: "NM".into(),
            quantite: 2,
            artwork: 1,
            complement,
        }
    }

    /// Un export Scanflip garde ses onze colonnes au caractère près ; un
    /// export « Application » ajoute les siennes après, et elles se relisent.
    #[test]
    fn les_colonnes_application_s_ajoutent_et_se_relisent() {
        let scanflip = ecrire_octets(&[sortie_application(None)]).unwrap();
        let entete = String::from_utf8(scanflip[3..].to_vec()).unwrap();
        assert!(
            entete.starts_with(&format!("{}\r\n", COLONNES.join(","))),
            "{entete}"
        );
        let relu = lire_octets(&scanflip).unwrap();
        assert!(!relu.application);
        assert_eq!(relu.lignes[0].complement, None);

        let complement = Complement {
            rarete: "Quarter Century Secret Rare".into(),
            nom_anglais: "Blue-Eyes White Dragon".into(),
            illustration: "BlueEyesWhiteDragon-RA04-EN-QCScR-1E.png".into(),
            id_image: Some(-1_589_717_855),
            overframe: true,
        };
        let application = ecrire_octets(&[
            sortie_application(Some(complement.clone())),
            // Une ligne sans complément dans un fichier qui en porte : ses
            // cases restent vides.
            sortie_application(None),
        ])
        .unwrap();
        let texte = String::from_utf8(application[3..].to_vec()).unwrap();
        let premiere = texte.lines().next().unwrap();
        assert_eq!(
            premiere,
            format!("{},{}", COLONNES.join(","), COLONNES_APPLICATION.join(","))
        );
        let relu = lire_octets(&application).unwrap();
        assert!(relu.application);
        assert!(relu.illisibles.is_empty());
        assert_eq!(relu.lignes[0].complement.as_ref(), Some(&complement));
        assert_eq!(
            relu.lignes[0].artwork, 1,
            "les onze colonnes n'ont pas bougé"
        );
        assert_eq!(relu.lignes[0].quantite, 2);
        assert_eq!(
            relu.lignes[1].complement,
            Some(Complement::default()),
            "colonnes présentes, cases vides"
        );
    }

    /// Les cases du complément se lisent avec indulgence : un identifiant
    /// illisible compte comme absent, « Overframe » accepte ce qu'un tableur
    /// y écrit.
    #[test]
    fn les_cases_du_complement_se_lisent_avec_indulgence() {
        let csv = "Code,Rareté,ID image,Overframe\n\
                   RA04-EN002,QCR,-42,1\n\
                   RA04-EN002,QCR,quarante,oui\n\
                   RA04-EN002,QCR,,0\n\
                   RA04-EN002,QCR, 7 ,\n";
        let relu = lire_octets(csv.as_bytes()).unwrap();
        assert!(
            relu.application,
            "une seule colonne suffit à le reconnaître"
        );
        let c: Vec<(Option<i64>, bool)> = relu
            .lignes
            .iter()
            .map(|l| {
                let c = l.complement.as_ref().unwrap();
                (c.id_image, c.overframe)
            })
            .collect();
        assert_eq!(
            c,
            vec![
                (Some(-42), true),
                (None, true),
                (None, false),
                (Some(7), false)
            ]
        );
        for (texte, attendu) in [
            ("", false),
            ("0", false),
            ("Non", false),
            ("faux", false),
            ("1", true),
            ("x", true),
            ("Overframe", true),
        ] {
            assert_eq!(vrai(texte), attendu, "« {texte} »");
        }
    }
}
