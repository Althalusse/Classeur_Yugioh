// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Cardmarket — de la carte du classeur à sa page, et à son prix.
//!
//! # Le chemin, en quatre étapes
//!
//! 1. **Les fichiers publics** descendent au plus une fois par jour dans
//!    `bdd/cardmarket/` (`ygo_sources::cardmarket`).
//! 2. **L'index** (`bdd/cardmarket/index.json`) rapproche chaque extension
//!    Cardmarket d'un set de `cardinfo.db` — par les noms de cartes, indice de
//!    Jaccard ≥ 0,5, à défaut par le nom du set (un set OCG récent, aux noms
//!    de cartes provisoires) —, range les produits de chaque carte, et mesure
//!    le sens de saisie de l'extension ([`Sens`]). Il ne se refait que si un
//!    fichier ou la base a changé.
//! 3. **L'offre** d'une ligne de classeur ([`IndexCharge::offre`]) : la
//!    rareté donne le rang V.n (`ygo_core::cardmarket::ordonner`), le sens de
//!    saisie donne le produit.
//! 4. **L'adresse** : la page du produit quand on sait lequel c'est ; sinon la
//!    recherche de la carte dans son extension, qui liste chaque version avec
//!    son prix, et le rang V.n à ouvrir ; et quand la carte elle-même n'a pas
//!    été retrouvée, la recherche de son numéro ([`Cible::Numero`]).
//!
//! Le prix d'un exemplaire sûr chiffre aussi la collection : [`valoriser`]
//! pour l'accueil, [`prix_des_lignes`] pour un classeur ouvert.
//!
//! Deux lignes d'un numéro peuvent avoir la même rareté — deux illustrations
//! (RA02 « Rescue Cat »), une Ultra ordinaire et une Ultra Overframe (cartes
//! de légende de LOCH et LOCR). Le fichier d'image de chaque ligne les
//! sépare : [`cles_de_version`] donne à chacune sa clé (« Super Rare
//! (illustration 2) », « Ultra Rare (Overframe) »), et ce sont ces clés que
//! l'on ordonne.
//!
//! # Ce qui est sûr, ce qui ne l'est pas — mesuré en octobre 2026
//!
//! Le rang V.n est connu pour 79,6 % du catalogue (règle confirmée sur trente
//! sets sans contradiction). L'`idProduct` d'une version, lui, ne se déduit
//! de l'ordre des identifiants que si l'extension a été saisie dans un ordre
//! reconnaissable ([`Sens`]) : sur les 1 060 extensions rapprochées, TCG 71
//! dans l'ordre, 2 à l'envers (BLMM, AMDE), 5 mélangées ; OCG 55 dans l'ordre,
//! 86 mélangées ; les autres sans assez de prix pour trancher. Une extension
//! TCG dans ce dernier cas est **supposée** dans l'ordre (c'est le cas
//! général) et l'offre le dit ([`Raison::OrdreDeSaisieSuppose`]) ; une OCG ne
//! l'est pas.
//!
//! Même dans un set saisi dans l'ordre, une carte peut ne pas l'être (une
//! version ajoutée après coup au milieu des autres) : les prix de la carte
//! doivent alors coller à l'attribution ([`vraisemblable`]), sinon la fiche
//! se rabat sur la recherche.
//!
//! Contrôle sur les pages relevées par l'utilisateur (COTD, SAST, LIOV, DAMA,
//! BPRO, GS05, PHRE, BLMM, DUAD, 20TH-JPC, LOCH, LOCR, MAMA, MAGO) : 34 liens
//! directs, **34 justes** ; les 30 autres versions passent par la recherche,
//! avec le bon numéro V.n affiché quand il est connu. Par lignes de classeur,
//! illustrations et Overframe comprises (RA02 « Rescue Cat », RA05
//! « Raigeki », LOCR et LOCH « légendes ») : 37 liens directs, **37 justes** ;
//! RA04 « El Shaddoll Winda », illustrations saisies à part : 8 recherches,
//! chacune avec son bon V.n. Sur les 17 classeurs de l'utilisateur (4 516
//! lignes) : lien direct pour 93,7 % des lignes (dont 0,2 % sur ordre de
//! saisie supposé), 93,3 % des 1 325 lignes possédées ; 170 lignes, dont
//! 63 possédées, cherchent leur numéro (CORI surtout : noms provisoires,
//! numéros que la base ne connaît pas encore).
//!
//! Aucun prix n'est attribué au hasard : quand le produit n'est pas sûr,
//! l'offre donne la fourchette des versions de la carte, pas un prix.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use ygo_core::cardmarket::{self as regles, ContexteOrdre, Distinction, Region, Sens};
use ygo_core::paths::Paths;
use ygo_sources::cardmarket::{self as fichiers, Catalogue, Fichier, GuidePrix, PrixPublie};
use ygo_sources::yugipedia::artwork;
use ygo_sources::ClientHttp;

use crate::error::Result;

/// Version du format de l'index : un index d'une autre version est refait.
///
/// 2 (2026-10-21) : le sens de saisie se mesure au prix médian
/// ([`Prix::reference`]), plus à la seule tendance.
/// 3 (2026-10-22) : une extension que ses cartes ne rapprochent pas l'est par
/// le nom de son set (`rapprocher_par_le_nom` : CORI).
pub const VERSION_INDEX: u32 = 3;

/// Nom du fichier d'index.
pub const NOM_INDEX: &str = "index.json";

/// Indice de Jaccard minimal pour rapprocher une extension d'un set.
pub const JACCARD_MINIMUM: f64 = 0.5;

/// Une extension Cardmarket a besoin d'au moins ce nombre de cartes pour
/// être rapprochée : en deçà, deux noms en commun font un faux ami.
pub const CARTES_MINIMUM: usize = 3;

/// Les langues de `set_locales` dont on compare les cartes. Les autres
/// langues occidentales partagent l'extension TCG du set ; le coréen et le
/// chinois ont des extensions à part chez Cardmarket, non couvertes.
const LANGUES_RAPPROCHEES: [&str; 4] = ["en", "na", "eu", "jp"];

/// Le dossier des fichiers Cardmarket d'une installation.
#[must_use]
pub fn dossier(paths: &Paths) -> PathBuf {
    paths.bdd().join("cardmarket")
}

/// Le chemin de l'index.
#[must_use]
pub fn chemin_index(paths: &Paths) -> PathBuf {
    dossier(paths).join(NOM_INDEX)
}

// ─────────────────────────────────────────────────────────────────────────────
// L'index, tel qu'écrit sur le disque
// ─────────────────────────────────────────────────────────────────────────────

/// Ce dont l'index a été construit : si l'un change, il est à refaire.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Empreinte {
    /// `createdAt` du catalogue des cartes.
    pub cartes: String,
    /// `createdAt` des produits scellés.
    pub scelles: String,
    /// `createdAt` du guide des prix.
    pub prix: String,
    /// Taille et date de `cardinfo.db`.
    pub cardinfo: String,
}

/// Les prix d'un produit, **en centimes** — des entiers, pour que l'offre
/// reste comparable et que l'arrondi soit fait une fois, ici.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prix {
    /// Tendance.
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "t")]
    pub tendance: Option<u32>,
    /// Plus bas prix proposé.
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "b")]
    pub bas: Option<u32>,
    /// Moyenne des ventes sur trente jours.
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "m")]
    pub moyenne30: Option<u32>,
}

impl Prix {
    /// Depuis le guide publié, en euros.
    #[must_use]
    pub fn de(publie: &PrixPublie) -> Self {
        Self {
            tendance: centimes(publie.trend),
            bas: centimes(publie.low),
            moyenne30: centimes(publie.avg30),
        }
    }

    /// Le prix qui représente le produit pour les comparaisons : le
    /// **médian** de la tendance, du plus bas et de la moyenne sur trente
    /// jours — de deux, leur moyenne ; d'un seul, lui.
    ///
    /// Une de ces valeurs peut être aberrante : la tendance d'un produit peu
    /// vendu s'effondre parfois (LOCR « Revolution Cyber Dragon », Secret
    /// Rare : tendance 0,02 €, plus bas 0,50 €, moyenne 3,88 € — 10 % des
    /// produits ont une tendance sous leur plus bas), un plus bas peut venir
    /// d'un exemplaire abîmé. Le médian l'écarte.
    ///
    /// ```
    /// use ygo_app::cardmarket::Prix;
    /// let p = Prix { tendance: Some(2), bas: Some(50), moyenne30: Some(388) };
    /// assert_eq!(p.reference(), Some(50));
    /// let p = Prix { tendance: Some(100), bas: Some(51), moyenne30: None };
    /// assert_eq!(p.reference(), Some(75));
    /// assert_eq!(Prix { bas: Some(750_000), ..Prix::default() }.reference(), Some(750_000));
    /// assert_eq!(Prix::default().reference(), None);
    /// ```
    #[must_use]
    pub fn reference(&self) -> Option<u32> {
        let mut v: Vec<u32> = [self.tendance, self.bas, self.moyenne30]
            .into_iter()
            .flatten()
            .collect();
        v.sort_unstable();
        match v.as_slice() {
            [x] | [_, x, _] => Some(*x),
            [a, b] => Some(a / 2 + b / 2 + (a % 2 + b % 2) / 2),
            _ => None,
        }
    }

    /// Aucun prix connu ?
    #[must_use]
    pub fn est_vide(&self) -> bool {
        self.tendance.is_none() && self.bas.is_none() && self.moyenne30.is_none()
    }
}

/// Des euros aux centimes ; rien pour une valeur absente, négative ou absurde.
fn centimes(euros: Option<f64>) -> Option<u32> {
    let e = euros?;
    if !e.is_finite() || !(0.0..=10_000_000.0).contains(&e) {
        return None;
    }
    let c = (e * 100.0).round();
    // Borné ci-dessus : la conversion est exacte.
    Some(c as u32)
}

/// Les produits d'une carte dans une extension.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Versions {
    /// Ceux créés avec le set, par identifiant croissant.
    pub ids: Vec<i64>,
    /// Ceux ajoutés plus tard (rééditions, cartes géantes), que la base ne
    /// connaît pas.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ajouts: Vec<i64>,
}

/// Une extension Cardmarket rapprochée d'un set de la base.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Extension {
    /// `idExpansion`.
    pub id: i64,
    /// Son nom chez Cardmarket, déduit de ses produits scellés.
    #[serde(default)]
    pub libelle: Option<String>,
    /// Le set de la base.
    pub set_uuid: String,
    /// La langue de la locale rapprochée (`en`, `na`, `eu`, `jp`).
    pub langue: String,
    /// Son préfixe (`RA04-EN`).
    pub prefixe: String,
    /// TCG ou OCG.
    pub region: Region,
    /// Date de sortie de la locale rapprochée.
    #[serde(default)]
    pub sortie: Option<String>,
    /// Indice de Jaccard du rapprochement.
    pub jaccard: f64,
    /// Sens de saisie des produits.
    pub sens: Sens,
    /// Témoins du sens : (dans l'ordre, à l'envers).
    pub temoins: (u32, u32),
    /// Les produits de chaque carte, par nom normalisé.
    pub cartes: BTreeMap<String, Versions>,
}

impl Extension {
    /// Le contexte d'ordre de l'extension.
    #[must_use]
    pub fn contexte(&self) -> ContexteOrdre<'_> {
        ContexteOrdre {
            id_extension: self.id,
            region: self.region,
            sortie: self.sortie.as_deref(),
        }
    }
}

/// L'index complet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Index {
    /// [`VERSION_INDEX`].
    pub version: u32,
    /// Ce dont il a été construit.
    pub empreinte: Empreinte,
    /// Quand (secondes Unix).
    pub construit_le: i64,
    /// Les extensions rapprochées, par identifiant.
    pub extensions: Vec<Extension>,
    /// Extensions Cardmarket que rien n'a rapprochées.
    pub non_rapprochees: usize,
    /// Les prix des produits des extensions rapprochées.
    pub prix: HashMap<i64, Prix>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Construction
// ─────────────────────────────────────────────────────────────────────────────

/// Une locale de la base.
#[derive(Debug, Clone)]
struct Locale {
    set_uuid: String,
    langue: String,
    prefixe: String,
    sortie: Option<String>,
}

/// Numéro → raretés, pour une carte d'une locale.
pub type Numeros = BTreeMap<String, BTreeSet<String>>;

/// Nom normalisé → numéros, pour une locale.
pub type CartesDeLocale = HashMap<String, Numeros>;

/// Les produits d'une extension Cardmarket, par nom normalisé : (identifiant,
/// date d'ajout).
type ProduitsParNom<'a> = BTreeMap<String, Vec<(i64, &'a str)>>;

/// La clé de classement d'un candidat au rapprochement : indice de Jaccard
/// au centième, même région, écart de taille (négatif), locale.
type CleCandidat = (i64, bool, i64, i64);

/// Ce que la base sait des sets, réduit à ce que le rapprochement compare.
#[derive(Debug, Default)]
pub struct Base {
    locales: HashMap<i64, Locale>,
    /// Locale → nom normalisé → numéro → raretés.
    tirages: HashMap<i64, CartesDeLocale>,
    /// Set → son nom anglais, normalisé (cf. `rapprocher_par_le_nom`).
    noms_de_sets: HashMap<String, String>,
}

impl Base {
    /// Lit `cardinfo.db`.
    ///
    /// # Errors
    ///
    /// Si une table ne se lit pas.
    pub fn lire(conn: &Connection) -> Result<Self> {
        let mut noms: HashMap<String, String> = HashMap::new();
        let mut q = conn.prepare("SELECT card_uuid, name FROM card_texts WHERE language = 'en'")?;
        let lignes = q.query_map([], |l| {
            Ok((l.get::<_, String>(0)?, l.get::<_, Option<String>>(1)?))
        })?;
        for l in lignes {
            let (uuid, nom) = l?;
            if let Some(nom) = nom.filter(|n| !n.trim().is_empty()) {
                noms.insert(uuid, regles::normaliser_nom(&nom));
            }
        }

        let mut locales = HashMap::new();
        let mut q = conn.prepare(
            "SELECT id, set_uuid, language, COALESCE(prefix, ''), release_date FROM set_locales",
        )?;
        let lignes = q.query_map([], |l| {
            Ok((
                l.get::<_, i64>(0)?,
                Locale {
                    set_uuid: l.get(1)?,
                    langue: l.get(2)?,
                    prefixe: l.get(3)?,
                    sortie: l
                        .get::<_, Option<String>>(4)?
                        .filter(|d| !d.trim().is_empty()),
                },
            ))
        })?;
        for l in lignes {
            let (id, locale) = l?;
            if LANGUES_RAPPROCHEES.contains(&locale.langue.as_str()) {
                locales.insert(id, locale);
            }
        }

        let mut tirages: HashMap<i64, CartesDeLocale> = HashMap::new();
        let mut q = conn.prepare(
            "SELECT set_locale_id, card_uuid, COALESCE(set_code, ''), COALESCE(rarity, '') \
             FROM set_prints",
        )?;
        let lignes = q.query_map([], |l| {
            Ok((
                l.get::<_, i64>(0)?,
                l.get::<_, String>(1)?,
                l.get::<_, String>(2)?,
                l.get::<_, String>(3)?,
            ))
        })?;
        for l in lignes {
            let (sl, uuid, code, rarete) = l?;
            if rarete.trim().is_empty() || !locales.contains_key(&sl) {
                continue;
            }
            let Some(nom) = noms.get(&uuid) else {
                continue;
            };
            tirages
                .entry(sl)
                .or_default()
                .entry(nom.clone())
                .or_default()
                .entry(code)
                .or_default()
                .insert(rarete);
        }

        // Les noms des sets : facultatifs, une base d'un autre schéma s'en
        // passe (le rapprochement par les cartes reste).
        let mut noms_de_sets = HashMap::new();
        if let Ok(mut q) = conn.prepare("SELECT uuid, name_en FROM sets") {
            let lignes = q.query_map([], |l| {
                Ok((l.get::<_, String>(0)?, l.get::<_, Option<String>>(1)?))
            })?;
            for l in lignes {
                let (uuid, nom) = l?;
                if let Some(nom) = nom.filter(|n| !n.trim().is_empty()) {
                    noms_de_sets.insert(uuid, regles::normaliser_nom(&nom));
                }
            }
        }
        Ok(Self {
            locales,
            tirages,
            noms_de_sets,
        })
    }
}

/// Reconstruit l'index depuis les fichiers et la base.
///
/// # Errors
///
/// Si le catalogue des cartes ou la base ne se lisent pas. Les produits
/// scellés et le guide des prix sont facultatifs : sans eux, l'index n'a ni
/// noms d'extensions, ni prix, ni sens de saisie.
pub fn construire(paths: &Paths) -> Result<Index> {
    let d = dossier(paths);
    let cartes = fichiers::lire_catalogue(&fichiers::chemin(&d, Fichier::Cartes))?;
    let scelles =
        fichiers::lire_catalogue(&fichiers::chemin(&d, Fichier::Scelles)).unwrap_or_default();
    let guide = fichiers::lire_prix(&fichiers::chemin(&d, Fichier::Prix)).unwrap_or_default();
    let conn = ygo_db::connexion::ouvrir_lecture_seule(paths.cardinfo_db())?;
    let base = Base::lire(&conn)?;
    Ok(assembler(
        &cartes,
        &scelles,
        &guide,
        &base,
        empreinte_courante(paths),
    ))
}

/// Ce que l'index **serait** s'il était refait maintenant.
#[must_use]
pub fn empreinte_courante(paths: &Paths) -> Empreinte {
    let d = dossier(paths);
    let date = |f: Fichier| fichiers::date_du_fichier(&fichiers::chemin(&d, f)).unwrap_or_default();
    let cardinfo = std::fs::metadata(paths.cardinfo_db())
        .map(|m| {
            let date = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs());
            format!("{}-{date}", m.len())
        })
        .unwrap_or_default();
    Empreinte {
        cartes: date(Fichier::Cartes),
        scelles: date(Fichier::Scelles),
        prix: date(Fichier::Prix),
        cardinfo,
    }
}

/// Le cœur de la construction, sans fichier ni base : testable tel quel.
#[must_use]
pub fn assembler(
    cartes: &Catalogue,
    scelles: &Catalogue,
    guide: &GuidePrix,
    base: &Base,
    empreinte: Empreinte,
) -> Index {
    // Les produits Cardmarket, par extension puis par nom normalisé.
    let mut cm: BTreeMap<i64, ProduitsParNom<'_>> = BTreeMap::new();
    for p in &cartes.products {
        cm.entry(p.id_extension)
            .or_default()
            .entry(regles::normaliser_nom(&p.name))
            .or_default()
            .push((p.id, p.ajoute_le.as_str()));
    }
    let mut noms_scelles: HashMap<i64, Vec<&str>> = HashMap::new();
    for p in &scelles.products {
        noms_scelles
            .entry(p.id_extension)
            .or_default()
            .push(p.name.as_str());
    }
    let prix_publies: HashMap<i64, Prix> = guide
        .prix
        .iter()
        .map(|p| (p.id, Prix::de(p)))
        .filter(|(_, p)| !p.est_vide())
        .collect();

    // Index inverse : nom → locales qui l'ont ; et nombre de tirages par locale.
    let mut inverse: HashMap<&str, Vec<i64>> = HashMap::new();
    let mut tirages_par_locale: HashMap<i64, usize> = HashMap::new();
    for (sl, cartes_locale) in &base.tirages {
        let mut n = 0;
        for (nom, codes) in cartes_locale {
            inverse.entry(nom.as_str()).or_default().push(*sl);
            n += codes.values().map(BTreeSet::len).sum::<usize>();
        }
        tirages_par_locale.insert(*sl, n);
    }

    let mut extensions = Vec::new();
    let mut non_rapprochees = 0;
    let mut ids_retenus: HashSet<i64> = HashSet::new();
    for (id_ext, noms) in &cm {
        let ocg = noms_scelles
            .get(id_ext)
            .is_some_and(|v| v.iter().any(|n| est_libelle_ocg(n)));
        let scelles_de_l_ext = noms_scelles.get(id_ext).map_or(&[][..], Vec::as_slice);
        let libelle = libelle_extension(scelles_de_l_ext);
        let rapprochee = rapprocher(noms, ocg, &inverse, &tirages_par_locale, base)
            .or_else(|| rapprocher_par_le_nom(noms, libelle.as_deref(), scelles_de_l_ext, base));
        let Some((sl, jaccard)) = rapprochee else {
            non_rapprochees += 1;
            continue;
        };
        let Some(locale) = base.locales.get(&sl) else {
            non_rapprochees += 1;
            continue;
        };
        let region = if locale.langue == "jp" {
            Region::Ocg
        } else {
            Region::Tcg
        };
        let mut ext = Extension {
            id: *id_ext,
            libelle,
            set_uuid: locale.set_uuid.clone(),
            langue: locale.langue.clone(),
            prefixe: locale.prefixe.clone(),
            region,
            sortie: locale.sortie.clone(),
            jaccard,
            sens: Sens::Inconnu,
            temoins: (0, 0),
            cartes: BTreeMap::new(),
        };
        for (nom, produits) in noms {
            let (ids, ajouts) = regles::separer_ajouts(produits);
            ids_retenus.extend(ids.iter().chain(&ajouts));
            ext.cartes.insert(nom.clone(), Versions { ids, ajouts });
        }
        let temoins = temoins_du_sens(&ext, base.tirages.get(&sl), &prix_publies);
        ext.temoins = temoins;
        ext.sens = regles::classer_sens(temoins.0, temoins.1);
        extensions.push(ext);
    }
    let prix = prix_publies
        .into_iter()
        .filter(|(id, _)| ids_retenus.contains(id))
        .collect();
    Index {
        version: VERSION_INDEX,
        empreinte,
        construit_le: fichiers::maintenant(),
        extensions,
        non_rapprochees,
        prix,
    }
}

/// La locale de la base la plus proche d'une extension, et son indice de
/// Jaccard — `None` en deçà de [`JACCARD_MINIMUM`].
///
/// À indice égal (au centième), on préfère la locale de la même région, puis
/// celle dont le nombre de tirages est le plus proche du nombre de produits.
fn rapprocher(
    noms: &ProduitsParNom<'_>,
    ocg: bool,
    inverse: &HashMap<&str, Vec<i64>>,
    tirages_par_locale: &HashMap<i64, usize>,
    base: &Base,
) -> Option<(i64, f64)> {
    if noms.len() < CARTES_MINIMUM {
        return None;
    }
    let mut communs: HashMap<i64, usize> = HashMap::new();
    for nom in noms.keys() {
        for sl in inverse.get(nom.as_str()).into_iter().flatten() {
            *communs.entry(*sl).or_default() += 1;
        }
    }
    let produits = noms.values().map(Vec::len).sum::<usize>();
    let mut meilleur: Option<(CleCandidat, i64, f64)> = None;
    for (sl, inter) in communs {
        let Some(cartes_locale) = base.tirages.get(&sl) else {
            continue;
        };
        let union = noms.len() + cartes_locale.len() - inter;
        if union == 0 {
            continue;
        }
        let j = inter as f64 / union as f64;
        let meme_region = base
            .locales
            .get(&sl)
            .is_some_and(|l| (l.langue == "jp") == ocg);
        let ecart = tirages_par_locale
            .get(&sl)
            .copied()
            .unwrap_or(0)
            .abs_diff(produits);
        let cle = (
            (j * 100.0).round() as i64,
            meme_region,
            -i64::try_from(ecart).unwrap_or(i64::MAX),
            sl,
        );
        if meilleur.as_ref().is_none_or(|(c, _, _)| cle > *c) {
            meilleur = Some((cle, sl, j));
        }
    }
    let (_, sl, j) = meilleur?;
    (j >= JACCARD_MINIMUM).then_some((sl, (j * 1000.0).round() / 1000.0))
}

/// Le rapprochement par le **nom du set**, quand les cartes ne suffisent pas.
///
/// Un set OCG récent n'a dans la base qu'une partie de ses cartes, sous des
/// noms anglais provisoires : CORI (« Chaos Origins », avril 2026) y a 48
/// cartes quand Cardmarket en a 96, « Black Chaos the Ultimate Magical
/// Swordsman » pour « Black Chaos » — indice de Jaccard 0,24, sous le seuil.
/// Son nom, lui, ne trompe pas : l'extension s'appelle « Chaos Origins
/// (OCG) » (nom tiré de ses produits scellés), le set « Chaos Origins ».
///
/// Il faut le même nom, la locale de la même région — japonaise pour une
/// extension « (OCG) » ou « (Japanese) », occidentale sans mention — et au
/// moins [`CARTES_MINIMUM`] cartes en commun. Une extension coréenne ou
/// asiatique n'est jamais rapprochée ainsi : ses cartes ont les noms
/// anglais du set japonais, mais pas ses produits.
fn rapprocher_par_le_nom(
    noms: &ProduitsParNom<'_>,
    libelle: Option<&str>,
    scelles: &[&str],
    base: &Base,
) -> Option<(i64, f64)> {
    const JAPONAISES: [&str; 2] = ["(OCG", "(Japanese"];
    let libelle = libelle?;
    let mention = |marques: &[&str]| {
        scelles
            .iter()
            .any(|n| marques.iter().any(|m| n.contains(m)))
    };
    if mention(&["(Korean", "(Asian"]) {
        return None;
    }
    let ocg = mention(&JAPONAISES);
    let nom_du_set = JAPONAISES
        .iter()
        .find_map(|m| libelle.find(&format!(" {m}")))
        .and_then(|i| libelle.get(..i))
        .unwrap_or(libelle);
    let cle = regles::normaliser_nom(nom_du_set);
    if cle.is_empty() {
        return None;
    }
    let mut meilleur: Option<(usize, i64, f64)> = None;
    for (sl, locale) in &base.locales {
        if (locale.langue == "jp") != ocg || base.noms_de_sets.get(&locale.set_uuid) != Some(&cle) {
            continue;
        }
        let Some(cartes_locale) = base.tirages.get(sl) else {
            continue;
        };
        let inter = noms
            .keys()
            .filter(|n| cartes_locale.contains_key(*n))
            .count();
        if inter < CARTES_MINIMUM {
            continue;
        }
        let union = noms.len() + cartes_locale.len() - inter;
        #[allow(clippy::cast_precision_loss)]
        let j = inter as f64 / union.max(1) as f64;
        // À égalité, la locale de plus petit identifiant : un ordre stable.
        if meilleur.is_none_or(|(i, s, _)| inter > i || (inter == i && *sl < s)) {
            meilleur = Some((inter, *sl, j));
        }
    }
    let (_, sl, j) = meilleur?;
    Some((sl, (j * 1000.0).round() / 1000.0))
}

/// Les produits scellés d'une extension OCG le disent dans leur nom.
fn est_libelle_ocg(nom: &str) -> bool {
    ["(OCG", "(Japanese", "(Korean", "(Asian"]
        .iter()
        .any(|m| nom.contains(m))
}

/// Le nom d'une extension, tiré du plus court de ses produits scellés
/// débarrassé de « Booster Box », « Display »…
fn libelle_extension(noms: &[&str]) -> Option<String> {
    const SUFFIXES: [&str; 14] = [
        " Booster Box",
        " Booster",
        " Display",
        " Case",
        " Box Set",
        " Card Pack",
        " Bundle",
        " Blister",
        " Tin",
        " Box",
        ": Special Edition",
        " Special Edition",
        " +1 Bonus Pack",
        " Deck",
    ];
    noms.iter()
        .filter_map(|n| {
            let mut x = n.trim();
            // « 24x Booster » : le nombre devant ne fait pas partie du nom.
            if let Some((tete, reste)) = x.split_once("x ") {
                if !tete.is_empty() && tete.chars().all(|c| c.is_ascii_digit()) {
                    x = reste;
                }
            }
            for s in SUFFIXES {
                if let Some(i) = x.find(s) {
                    x = x.get(..i).unwrap_or(x);
                }
            }
            let x = x.trim().trim_matches(['-', ':', ' ']);
            (!x.is_empty()).then(|| x.to_owned())
        })
        .min_by_key(String::len)
}

/// Les témoins du sens de saisie : pour chaque carte dont les versions ont
/// un ordre connu, la première paire très écartée en valeur — la plus
/// précieuse a-t-elle l'identifiant le plus grand ?
fn temoins_du_sens(
    ext: &Extension,
    tirages: Option<&CartesDeLocale>,
    prix: &HashMap<i64, Prix>,
) -> (u32, u32) {
    let Some(tirages) = tirages else {
        return (0, 0);
    };
    let ctx = ext.contexte();
    let (mut dans_l_ordre, mut a_l_envers) = (0_u32, 0_u32);
    for (nom, versions) in &ext.cartes {
        let Some(codes) = tirages.get(nom) else {
            continue;
        };
        if codes.len() != 1 || !versions.ajouts.is_empty() || versions.ids.len() < 2 {
            continue;
        }
        let raretes: Vec<String> = codes.values().flatten().cloned().collect();
        if raretes.len() != versions.ids.len() {
            continue;
        }
        let Some(ordre) = regles::ordonner(&raretes, &ctx) else {
            continue;
        };
        // La paire la plus écartée en valeur : entre une Super et une
        // Secret, deux cartes à 0,02 € ne disent rien ; entre une Super et
        // une Prismatic, le prix parle.
        let mut meilleure: Option<(f64, u32, u32)> = None;
        for (i, a) in ordre.iter().enumerate() {
            for (j, b) in ordre.iter().enumerate().skip(i + 1) {
                let (Some(va), Some(vb)) = (regles::valeur(a), regles::valeur(b)) else {
                    continue;
                };
                if vb - va < regles::ECART_DE_VALEUR {
                    continue;
                }
                let pa = versions
                    .ids
                    .get(i)
                    .and_then(|id| prix.get(id))
                    .and_then(Prix::reference);
                let pb = versions
                    .ids
                    .get(j)
                    .and_then(|id| prix.get(id))
                    .and_then(Prix::reference);
                if let (Some(pa), Some(pb)) = (pa, pb) {
                    if meilleure.is_none_or(|(e, _, _)| vb - va > e) {
                        meilleure = Some((vb - va, pa, pb));
                    }
                }
            }
        }
        let Some((_, pa, pb)) = meilleure else {
            continue;
        };
        // Un écart de quelques centimes est du bruit, pas un témoignage.
        if pa.abs_diff(pb) < ECART_DE_PRIX_SIGNIFICATIF {
            continue;
        }
        if pb > pa {
            dans_l_ordre += 1;
        } else {
            a_l_envers += 1;
        }
    }
    (dans_l_ordre, a_l_envers)
}

// ─────────────────────────────────────────────────────────────────────────────
// Lecture, écriture, mémoire
// ─────────────────────────────────────────────────────────────────────────────

/// Écrit l'index, d'un renommage.
///
/// # Errors
///
/// Si le dossier n'accepte pas l'écriture.
pub fn ecrire(paths: &Paths, index: &Index) -> Result<()> {
    let d = dossier(paths);
    std::fs::create_dir_all(&d)?;
    let octets = serde_json::to_vec(index)?;
    let mut tmp = tempfile::NamedTempFile::new_in(&d)?;
    std::io::Write::write_all(&mut tmp, &octets)?;
    tmp.persist(chemin_index(paths)).map_err(|e| e.error)?;
    Ok(())
}

/// Relit l'index ; `None` s'il manque, ne se lit pas ou date d'un autre
/// format.
#[must_use]
pub fn lire(paths: &Paths) -> Option<Index> {
    let octets = std::fs::read(chemin_index(paths)).ok()?;
    let index: Index = serde_json::from_slice(&octets).ok()?;
    (index.version == VERSION_INDEX).then_some(index)
}

/// L'index en mémoire, avec ses tables de recherche.
#[derive(Debug)]
pub struct IndexCharge {
    /// L'index.
    pub index: Index,
    /// (set, région) → extensions, meilleur rapprochement d'abord.
    par_set: HashMap<(String, Region), Vec<usize>>,
}

impl IndexCharge {
    /// Prépare les tables de recherche.
    #[must_use]
    pub fn new(index: Index) -> Self {
        let mut par_set: HashMap<(String, Region), Vec<usize>> = HashMap::new();
        for (i, e) in index.extensions.iter().enumerate() {
            par_set
                .entry((e.set_uuid.clone(), e.region))
                .or_default()
                .push(i);
        }
        for v in par_set.values_mut() {
            v.sort_by(|a, b| {
                let ja = index.extensions.get(*a).map_or(0.0, |e| e.jaccard);
                let jb = index.extensions.get(*b).map_or(0.0, |e| e.jaccard);
                jb.total_cmp(&ja)
            });
        }
        Self { index, par_set }
    }

    /// Les extensions rapprochées d'un set, dans une région.
    pub fn extensions_du_set(&self, set_uuid: &str, region: Region) -> Vec<&Extension> {
        self.par_set
            .get(&(set_uuid.to_owned(), region))
            .into_iter()
            .flatten()
            .filter_map(|i| self.index.extensions.get(*i))
            .collect()
    }
}

/// Les index chargés, par chemin, avec la date du fichier lu.
type Charges = HashMap<PathBuf, (SystemTime, Arc<IndexCharge>)>;

fn charges() -> &'static Mutex<Charges> {
    static CHARGES: OnceLock<Mutex<Charges>> = OnceLock::new();
    CHARGES.get_or_init(Mutex::default)
}

/// L'index de l'installation, chargé une fois puis gardé en mémoire tant que
/// le fichier ne change pas — `None` s'il n'y en a pas.
///
/// La première fiche ouverte paie la lecture (une fraction de seconde) ; le
/// fil Cardmarket du démarrage la paie d'ordinaire avant elle.
#[must_use]
pub fn charge(paths: &Paths) -> Option<Arc<IndexCharge>> {
    let chemin = chemin_index(paths);
    let date = std::fs::metadata(&chemin).and_then(|m| m.modified()).ok()?;
    let mut table = charges()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((d, i)) = table.get(&chemin) {
        if *d == date {
            return Some(Arc::clone(i));
        }
    }
    let index = lire(paths)?;
    let charge = Arc::new(IndexCharge::new(index));
    table.insert(chemin, (date, Arc::clone(&charge)));
    Some(charge)
}

// ─────────────────────────────────────────────────────────────────────────────
// Mise à jour
// ─────────────────────────────────────────────────────────────────────────────

/// Ce que l'index est devenu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EtatIndex {
    /// Il correspondait déjà aux fichiers et à la base.
    AJour,
    /// Il a été refait.
    Reconstruit {
        /// Extensions rapprochées.
        extensions: usize,
        /// Produits ayant un prix.
        produits: usize,
    },
    /// Rien à construire : il manque le catalogue ou la base.
    Impossible(String),
}

/// Le bilan d'une actualisation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bilan {
    /// Ce que les fichiers ont donné.
    pub fichiers: fichiers::Bilan,
    /// Ce que l'index est devenu.
    pub index: EtatIndex,
}

/// Vérifie les fichiers (une fois par jour au plus), puis refait l'index
/// s'il ne correspond plus aux fichiers ou à la base.
///
/// # Errors
///
/// Si le dossier ne peut pas être créé, ou si la construction échoue.
pub async fn actualiser(paths: &Paths, client: &ClientHttp, forcer: bool) -> Result<Bilan> {
    let fichiers = fichiers::telecharger(client, &dossier(paths), forcer).await?;
    let index = reconstruire_si_besoin(paths, false)?;
    Ok(Bilan { fichiers, index })
}

/// Refait l'index s'il ne correspond plus — ou toujours, avec `forcer`.
///
/// # Errors
///
/// Si la lecture des fichiers ou de la base, ou l'écriture, échoue.
pub fn reconstruire_si_besoin(paths: &Paths, forcer: bool) -> Result<EtatIndex> {
    if !fichiers::chemin(&dossier(paths), Fichier::Cartes).is_file() {
        return Ok(EtatIndex::Impossible(
            "catalogue Cardmarket pas encore téléchargé".to_owned(),
        ));
    }
    if !paths.cardinfo_db().is_file() {
        return Ok(EtatIndex::Impossible("base des cartes absente".to_owned()));
    }
    if !forcer {
        let courante = empreinte_courante(paths);
        if charge(paths).is_some_and(|c| c.index.empreinte == courante) {
            return Ok(EtatIndex::AJour);
        }
    }
    let index = construire(paths)?;
    let etat = EtatIndex::Reconstruit {
        extensions: index.extensions.len(),
        produits: index.prix.len(),
    };
    ecrire(paths, &index)?;
    // Rechargé tout de suite : la prochaine fiche n'attend pas.
    let _ = charge(paths);
    Ok(etat)
}

// ─────────────────────────────────────────────────────────────────────────────
// L'offre d'une ligne de classeur
// ─────────────────────────────────────────────────────────────────────────────

/// La carte dont on cherche la page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Demande<'a> {
    /// Numéro du tirage (`RA04-FR001`).
    pub set_code: &'a str,
    /// La version de la ligne : sa rareté, telle que le classeur l'écrit —
    /// ou sa clé de version quand le numéro porte plusieurs lignes de cette
    /// rareté (« Super Rare (illustration 2) », cf. [`cles_de_version`]).
    pub rarete: &'a str,
    /// Nom anglais.
    pub nom: &'a str,
    /// `card_uuid`, s'il est connu.
    pub card_uuid: Option<&'a str>,
    /// Les versions que le **classeur** connaît à ce numéro — raretés, ou
    /// clés de version comme `rarete` —, vide si on ne les a pas lues.
    ///
    /// Elles passent devant celles de `cardinfo.db` : pour un set récent, la
    /// base peut n'avoir que des « Common » d'attente (RA05, octobre 2026 :
    /// 228 tirages, tous en Common), quand le classeur, complété par
    /// YGOPRODeck et Yugipedia, a les sept raretés de chaque carte. Et la
    /// base ne connaît pas les illustrations d'une même rareté.
    pub raretes: &'a [String],
}

/// Les versions qu'on connaît à une carte dans un set.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Connues {
    /// Selon `cardinfo.db`, par numéro, dans la locale rapprochée : des
    /// raretés.
    pub base: Numeros,
    /// Selon le classeur, pour le numéro de la ligne : des raretés, ou des
    /// clés de version.
    pub classeur: Vec<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Les versions d'un numéro dans le classeur
// ─────────────────────────────────────────────────────────────────────────────

/// Une ligne de classeur, ce qu'il en faut pour nommer sa version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LigneDeClasseur {
    /// `rowid` de la ligne.
    pub rowid: i64,
    /// Rareté, telle que le classeur l'écrit.
    pub rarete: String,
    /// Nom du fichier d'image : `RescueCat-RA02-EN-SR-1E-AA.png`, ou
    /// `14878871.jpg` pour une image YGOPRODeck.
    pub fichier: String,
    /// Ligne marquée « illustration étendue » (Overframe).
    pub overframe: bool,
}

/// Le nom du fichier d'image d'une ligne : celui que désigne
/// `card_image_uuid` quand il vient de Yugipedia, à défaut le dernier
/// segment de `card_image_url`.
///
/// ```
/// use ygo_app::cardmarket::fichier_de_ligne;
/// assert_eq!(
///     fichier_de_ligne("yugipedia:RescueCat-RA02-EN-SR-1E-AA.png", ""),
///     "RescueCat-RA02-EN-SR-1E-AA.png"
/// );
/// assert_eq!(
///     fichier_de_ligne(
///         "1e086d41-0cc9-4091-afdb-f6602f6b397f",
///         "https://ms.yugipedia.com//2/29/RescueCat-RA02-EN-SR-1E.png"
///     ),
///     "RescueCat-RA02-EN-SR-1E.png"
/// );
/// assert_eq!(
///     fichier_de_ligne("", "https://images.ygoprodeck.com/images/cards/89631139.jpg?v=2"),
///     "89631139.jpg"
/// );
/// ```
#[must_use]
pub fn fichier_de_ligne(card_image_uuid: &str, card_image_url: &str) -> String {
    if let Some((source, nom)) = card_image_uuid.split_once(':') {
        if source.starts_with("yugipedia") && !nom.trim().is_empty() {
            return nom.trim().to_owned();
        }
    }
    let sans_requete = card_image_url.split(['?', '#']).next().unwrap_or("");
    sans_requete
        .rsplit('/')
        .next()
        .unwrap_or("")
        .trim()
        .to_owned()
}

/// Les lignes d'un numéro dans un classeur, dans l'ordre du classeur.
///
/// # Errors
///
/// Si le classeur ne se lit pas.
pub fn lignes_du_numero(conn: &Connection, set_code: &str) -> Result<Vec<LigneDeClasseur>> {
    // Les classeurs anciens n'ont pas `extended_art` : la colonne absente
    // ferait échouer toute la requête.
    let colonnes = ygo_db::migrations::colonnes(conn, "cards")?;
    let expr_overframe = if colonnes.iter().any(|c| c == "extended_art") {
        "COALESCE(extended_art, 0)"
    } else {
        "0"
    };
    let sql = format!(
        "SELECT rowid, COALESCE(rarity, ''), COALESCE(card_image_uuid, ''), \
                COALESCE(card_image_url, ''), {expr_overframe} \
         FROM cards WHERE set_code = ?1 AND COALESCE(rarity, '') <> '' \
         ORDER BY sort_order, rowid"
    );
    let mut q = conn.prepare_cached(&sql)?;
    let lignes = q
        .query_map([set_code], |l| {
            let uuid: String = l.get(2)?;
            let url: String = l.get(3)?;
            Ok(LigneDeClasseur {
                rowid: l.get(0)?,
                rarete: l.get(1)?,
                fichier: fichier_de_ligne(&uuid, &url),
                overframe: l.get::<_, i64>(4)? != 0,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(lignes)
}

/// La clé de version de chaque ligne d'un numéro, dans l'ordre reçu (cf.
/// `ygo_core::cardmarket::cle_de_version`).
///
/// Une rareté portée par une seule ligne garde son nom. Plusieurs lignes de
/// même rareté se distinguent par le suffixe de leur fichier Yugipedia —
/// `-AA`, `-B`, `-2` : une autre illustration ; `-EA`, ou la marque
/// « illustration étendue » de la ligne : l'Overframe. Si deux d'entre elles
/// ne se distinguent pas (RA04 « Cyber Dragon Infinity » : deux lignes
/// Quarter Century, deux fichiers `-AA`), ou qu'un suffixe ne dit rien de
/// sûr, elles gardent toutes le nom de la rareté : Cardmarket aura plus de
/// versions que le classeur n'en nomme, et la fiche cherchera plutôt que de
/// deviner.
///
/// ```
/// use ygo_app::cardmarket::{cles_de_version, LigneDeClasseur};
/// let ligne = |rowid, rarete: &str, fichier: &str, overframe| LigneDeClasseur {
///     rowid, rarete: rarete.into(), fichier: fichier.into(), overframe,
/// };
/// let cles = cles_de_version("RA02-EN001", &[
///     ligne(1, "Super Rare", "RescueCat-RA02-EN-SR-1E.png", false),
///     ligne(2, "Super Rare", "RescueCat-RA02-EN-SR-1E-AA.png", false),
///     ligne(3, "Ultra Rare", "RescueCat-RA02-EN-UR-1E.png", false),
/// ]);
/// assert_eq!(cles, ["Super Rare", "Super Rare (illustration 2)", "Ultra Rare"]);
/// ```
#[must_use]
pub fn cles_de_version(set_code: &str, lignes: &[LigneDeClasseur]) -> Vec<String> {
    let prefixe = artwork::prefixe_set(set_code);
    let mut par_rarete: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, l) in lignes.iter().enumerate() {
        par_rarete.entry(l.rarete.as_str()).or_default().push(i);
    }
    let mut cles: Vec<String> = lignes.iter().map(|l| l.rarete.clone()).collect();
    for (rarete, indices) in &par_rarete {
        if indices.len() < 2 {
            continue;
        }
        let distinctions: Option<Vec<Distinction>> = indices
            .iter()
            .map(|&i| {
                let l = lignes.get(i)?;
                let d = Distinction::du_suffixe(&artwork::suffixe(&l.fichier, &prefixe))?;
                Some(if d == Distinction::Aucune && l.overframe {
                    Distinction::Overframe
                } else {
                    d
                })
            })
            .collect();
        let Some(distinctions) = distinctions else {
            continue;
        };
        let uniques: HashSet<Distinction> = distinctions.iter().copied().collect();
        if uniques.len() != distinctions.len() {
            continue;
        }
        for (&i, d) in indices.iter().zip(&distinctions) {
            if let Some(cle) = cles.get_mut(i) {
                *cle = regles::cle_de_version(rarete, *d);
            }
        }
    }
    cles
}

/// Les versions d'un numéro et celle d'une de ses lignes : de quoi remplir
/// une [`Demande`]. La ligne absente du numéro garde sa rareté.
///
/// ```
/// use ygo_app::cardmarket::{versions_de_la_ligne, LigneDeClasseur};
/// let ligne = |rowid, fichier: &str| LigneDeClasseur {
///     rowid, rarete: "Ultra Rare".into(), fichier: fichier.into(), overframe: false,
/// };
/// let lignes = [
///     ligne(7, "BlueEyesWhiteDragontheWhitePhantomBeast-LOCR-JP-UR.png"),
///     ligne(8, "BlueEyesWhiteDragontheWhitePhantomBeast-LOCR-JP-UR-EA.png"),
/// ];
/// let (cle, versions) = versions_de_la_ligne("LOCR-JP001", &lignes, 8, "Ultra Rare");
/// assert_eq!(cle, "Ultra Rare (Overframe)");
/// assert_eq!(versions, ["Ultra Rare", "Ultra Rare (Overframe)"]);
/// ```
#[must_use]
pub fn versions_de_la_ligne(
    set_code: &str,
    lignes: &[LigneDeClasseur],
    rowid: i64,
    rarete: &str,
) -> (String, Vec<String>) {
    let cles = cles_de_version(set_code, lignes);
    let cle = lignes
        .iter()
        .zip(&cles)
        .find(|(l, _)| l.rowid == rowid)
        .map_or_else(|| rarete.to_owned(), |(_, c)| c.clone());
    let mut versions: Vec<String> = Vec::new();
    for c in cles {
        if !versions.contains(&c) {
            versions.push(c);
        }
    }
    (cle, versions)
}

/// Où mène le lien.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cible {
    /// La page du produit.
    Produit(i64),
    /// La recherche de la carte, dans son extension si on la connaît.
    Recherche {
        /// Nom anglais.
        nom: String,
        /// L'extension.
        id_extension: Option<i64>,
    },
    /// La recherche du numéro (`CORI-JP001`), quand ni l'extension ni la
    /// carte n'ont été retrouvées : les noms anglais provisoires d'un set OCG
    /// récent ne sont pas ceux de Cardmarket, son numéro si (cf.
    /// `ygo_core::cardmarket::numero_de_recherche`).
    Numero(String),
}

/// Le prix montré.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrixOffre {
    /// Celui du produit.
    Exact(Prix),
    /// De la version la moins chère à la plus chère — le produit n'est pas sûr.
    Fourchette {
        /// Le plus bas des prix de référence.
        min: u32,
        /// Le plus haut.
        max: u32,
    },
    /// Aucun prix connu.
    Aucun,
}

/// Pourquoi le lien est direct, ou ne l'est pas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Raison {
    /// La carte n'a qu'une version dans l'extension.
    VersionUnique,
    /// Le rang V.n et le sens de saisie désignent le produit.
    Ordonnee,
    /// Le rang est sûr ; le sens de saisie, supposé (extension TCG sans
    /// assez de prix pour le vérifier).
    OrdreDeSaisieSuppose,
    /// Le rang est sûr, mais l'extension a été saisie dans le désordre.
    SaisieMelangee,
    /// Le rang est sûr, mais les prix de la carte contredisent l'ordre de
    /// saisie de son set — une version ajoutée après coup, typiquement.
    SaisieDouteuse,
    /// L'ordre des raretés de la carte n'est pas connu.
    OrdreDesVersionsInconnu,
    /// La carte a plusieurs versions de la même rareté (illustrations,
    /// Overframe), et leur ordre n'a pas été relevé pour ce set.
    PlusieursVersionsDeLaRarete,
    /// Le rang est sûr, mais le set a créé ses illustrations alternatives à
    /// part : l'ordre de saisie ne dit pas lequel des produits est l'originale
    /// (cf. `ygo_core::cardmarket::illustrations_saisies_a_part`).
    IllustrationsSaisiesAPart,
    /// Cardmarket n'a pas le même nombre de versions que nous.
    NombreDeVersions {
        /// Chez Cardmarket.
        cardmarket: usize,
        /// Dans le classeur, à défaut dans la base.
        base: usize,
    },
    /// La carte porte plusieurs numéros dans le set.
    PlusieursNumeros,
    /// La rareté de la ligne n'est pas parmi celles de la carte.
    RareteInconnue,
    /// L'extension est connue, la carte n'y est pas.
    CarteAbsente,
    /// Le set n'a pas été rapproché d'une extension Cardmarket.
    ExtensionInconnue,
    /// La base des cartes ne connaît pas le numéro de la ligne (une carte de
    /// promotion du set, un numéro saisi à la main).
    NumeroInconnu,
}

impl Raison {
    /// Une phrase pour l'utilisateur.
    #[must_use]
    pub fn explication(self) -> String {
        match self {
            Self::VersionUnique => "une seule version dans ce set".to_owned(),
            Self::Ordonnee => "version identifiée".to_owned(),
            Self::OrdreDeSaisieSuppose => {
                "version identifiée — ordre de saisie de ce set supposé".to_owned()
            }
            Self::SaisieMelangee => {
                "ce set a été saisi dans le désordre chez Cardmarket".to_owned()
            }
            Self::SaisieDouteuse => {
                "les prix de cette carte ne collent pas à l'ordre de saisie du set".to_owned()
            }
            Self::OrdreDesVersionsInconnu => "ordre des versions inconnu pour ce set".to_owned(),
            Self::PlusieursVersionsDeLaRarete => {
                "plusieurs versions de cette rareté (illustrations, Overframe) : leur ordre \
                 n'est pas relevé pour ce set"
                    .to_owned()
            }
            Self::IllustrationsSaisiesAPart => {
                "illustrations alternatives saisies à part chez Cardmarket : ouvrir la version \
                 indiquée"
                    .to_owned()
            }
            Self::NombreDeVersions { cardmarket, base } => {
                format!("{cardmarket} version(s) chez Cardmarket, {base} de notre côté")
            }
            Self::PlusieursNumeros => "la carte porte plusieurs numéros dans ce set".to_owned(),
            Self::RareteInconnue => "rareté absente des versions connues".to_owned(),
            Self::CarteAbsente => "carte introuvable dans ce set chez Cardmarket".to_owned(),
            Self::ExtensionInconnue => "set non rapproché d'une extension Cardmarket".to_owned(),
            Self::NumeroInconnu => "numéro absent de la base des cartes".to_owned(),
        }
    }

    /// La carte elle-même n'a pas été retrouvée chez Cardmarket — pas
    /// seulement sa version : aucun prix ne peut lui être donné ici.
    #[must_use]
    pub fn carte_non_retrouvee(self) -> bool {
        matches!(
            self,
            Self::CarteAbsente | Self::ExtensionInconnue | Self::NumeroInconnu
        )
    }
}

/// Ce que la fiche montre : un lien, un prix, et de quoi en juger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offre {
    /// Où mène le lien.
    pub cible: Cible,
    /// La version à ouvrir — « V.3 · Ultimate Rare » —, si on la connaît.
    pub version: Option<String>,
    /// Le prix.
    pub prix: PrixOffre,
    /// Pourquoi.
    pub raison: Raison,
    /// Le nom de l'extension chez Cardmarket.
    pub extension: Option<String>,
    /// Date de génération du guide des prix.
    pub prix_du: String,
}

impl Offre {
    /// L'adresse, dans la langue du site voulue (`fr`, `en`).
    #[must_use]
    pub fn url(&self, langue: &str) -> String {
        match &self.cible {
            Cible::Produit(id) => regles::url_produit(langue, *id),
            Cible::Recherche { nom, id_extension } => {
                regles::url_recherche(langue, nom, *id_extension)
            }
            Cible::Numero(numero) => regles::url_recherche_numero(langue, numero),
        }
    }

    /// Le lien mène-t-il à la page même du produit ?
    #[must_use]
    pub fn est_directe(&self) -> bool {
        matches!(self.cible, Cible::Produit(_))
    }

    /// Le prix, en clair — `None` s'il n'y en a pas.
    ///
    /// La tendance d'abord ; la moyenne sur trente jours quand la tendance
    /// manque ou tombe sous le plus bas proposé — une tendance périmée, qui
    /// ferait passer une carte à 35 € pour une carte à 2 centimes.
    ///
    /// ```
    /// use ygo_app::cardmarket::{Cible, Offre, Prix, PrixOffre, Raison};
    /// let mut o = Offre {
    ///     cible: Cible::Produit(1),
    ///     version: None,
    ///     prix: PrixOffre::Exact(Prix { tendance: Some(549), bas: Some(500), moyenne30: None }),
    ///     raison: Raison::VersionUnique,
    ///     extension: None,
    ///     prix_du: String::new(),
    /// };
    /// assert_eq!(o.prix_affiche().unwrap(), "5,49 € (tendance) · dès 5,00 €");
    /// o.prix = PrixOffre::Exact(Prix { tendance: Some(2), bas: Some(50), moyenne30: Some(388) });
    /// assert_eq!(o.prix_affiche().unwrap(), "3,88 € (moyenne 30 j) · dès 0,50 €");
    /// o.prix = PrixOffre::Fourchette { min: 70, max: 123_456 };
    /// assert_eq!(o.prix_affiche().unwrap(), "de 0,70 € à 1 234,56 € selon la version");
    /// ```
    #[must_use]
    pub fn prix_affiche(&self) -> Option<String> {
        match self.prix {
            PrixOffre::Exact(p) => {
                let mut parties = Vec::new();
                let tendance = p
                    .tendance
                    .filter(|t| p.bas.is_none_or(|b| *t >= b) || p.moyenne30.is_none());
                if let Some(t) = tendance {
                    parties.push(format!("{} (tendance)", euros(t.into())));
                } else if let Some(m) = p.moyenne30 {
                    parties.push(format!("{} (moyenne 30 j)", euros(m.into())));
                }
                if let Some(b) = p.bas {
                    parties.push(format!("dès {}", euros(b.into())));
                }
                (!parties.is_empty()).then(|| parties.join(" · "))
            }
            PrixOffre::Fourchette { min, max } if min == max => {
                Some(format!("{} selon la version", euros(min.into())))
            }
            PrixOffre::Fourchette { min, max } => Some(format!(
                "de {} à {} selon la version",
                euros(min.into()),
                euros(max.into())
            )),
            PrixOffre::Aucun => None,
        }
    }
}

/// Des centimes en euros, à la française : `1 234,56 €`.
///
/// ```
/// use ygo_app::cardmarket::euros;
/// assert_eq!(euros(7), "0,07 €");
/// assert_eq!(euros(123_456_789), "1 234 567,89 €");
/// ```
#[must_use]
pub fn euros(centimes: u64) -> String {
    format!("{},{:02} €", milliers(centimes / 100), centimes % 100)
}

/// Des centimes en euros ronds, pour une pastille : `1 235 €`.
///
/// ```
/// use ygo_app::cardmarket::euros_ronds;
/// assert_eq!(euros_ronds(123_450), "1 235 €");
/// assert_eq!(euros_ronds(49), "0 €");
/// ```
#[must_use]
pub fn euros_ronds(centimes: u64) -> String {
    format!("{} €", milliers(centimes.saturating_add(50) / 100))
}

/// Un entier, ses milliers séparés par une espace : `1 234 567`.
fn milliers(n: u64) -> String {
    let chiffres = n.to_string();
    let mut groupe = String::new();
    for (i, c) in chiffres.chars().enumerate() {
        if i > 0 && (chiffres.len() - i) % 3 == 0 {
            groupe.push(' ');
        }
        groupe.push(c);
    }
    groupe
}

/// L'offre d'une ligne, ou `None` s'il n'y a pas encore d'index.
///
/// # Errors
///
/// Si `cardinfo.db` ne se lit pas.
pub fn offre(paths: &Paths, conn: &Connection, demande: &Demande<'_>) -> Result<Option<Offre>> {
    let Some(index) = charge(paths) else {
        return Ok(None);
    };
    index.offre(conn, demande).map(Some)
}

impl IndexCharge {
    /// L'offre d'une ligne de classeur.
    ///
    /// # Errors
    ///
    /// Si `cardinfo.db` ne se lit pas.
    pub fn offre(&self, conn: &Connection, d: &Demande<'_>) -> Result<Offre> {
        let prix_du = self.index.empreinte.prix.clone();
        // La carte non retrouvée — set non rapproché, nom absent de
        // l'extension, numéro inconnu de la base : son numéro la retrouve
        // chez Cardmarket là où son nom échoue (les noms provisoires d'un set
        // OCG récent : « Black Chaos the Ultimate Magical Swordsman » pour
        // « Black Chaos » chez Cardmarket). Le nom, si le code ne s'écrit pas
        // en numéro.
        let numero = regles::numero_de_recherche(d.set_code);
        let recherche = |raison: Raison, ext: Option<&Extension>| Offre {
            cible: numero.clone().map_or_else(
                || Cible::Recherche {
                    nom: d.nom.to_owned(),
                    id_extension: ext.map(|e| e.id),
                },
                Cible::Numero,
            ),
            version: None,
            prix: PrixOffre::Aucun,
            raison,
            extension: ext.and_then(|e| e.libelle.clone()),
            prix_du: prix_du.clone(),
        };

        // Le set et la langue du tirage, et la carte qu'il porte.
        let uuid_demande = d.card_uuid.unwrap_or("").trim();
        let ligne: Option<(String, String, String)> = conn
            .query_row(
                "SELECT sl.set_uuid, sl.language, sp.card_uuid \
                 FROM set_prints sp JOIN set_locales sl ON sl.id = sp.set_locale_id \
                 WHERE sp.set_code = ?1 \
                 ORDER BY (sp.card_uuid = ?2) DESC, sp.rowid LIMIT 1",
                params![d.set_code, uuid_demande],
                |l| Ok((l.get(0)?, l.get(1)?, l.get(2)?)),
            )
            .optional()?;
        let Some((set_uuid, langue, uuid_du_code)) = ligne else {
            return Ok(recherche(Raison::NumeroInconnu, None));
        };
        let card_uuid = if uuid_demande.is_empty() {
            uuid_du_code
        } else {
            uuid_demande.to_owned()
        };
        let Some(region) = Region::de_langue(&langue) else {
            return Ok(recherche(Raison::ExtensionInconnue, None));
        };
        let candidates = self.extensions_du_set(&set_uuid, region);
        if candidates.is_empty() {
            return Ok(recherche(Raison::ExtensionInconnue, None));
        }

        // Le nom anglais de la base d'abord : c'est lui qui a servi au
        // rapprochement ; celui du classeur à défaut.
        let nom_base: Option<String> = conn
            .query_row(
                "SELECT name FROM card_texts WHERE card_uuid = ?1 AND language = 'en'",
                [&card_uuid],
                |l| l.get(0),
            )
            .optional()?
            .flatten();
        let mut cles = Vec::new();
        if let Some(n) = &nom_base {
            cles.push(regles::normaliser_nom(n));
        }
        cles.push(regles::normaliser_nom(d.nom));
        let trouve = candidates
            .iter()
            .find_map(|e| cles.iter().find_map(|c| e.cartes.get(c)).map(|v| (*e, v)));
        let Some((ext, versions)) = trouve else {
            return Ok(recherche(Raison::CarteAbsente, candidates.first().copied()));
        };

        // Les raretés de la carte dans la locale rapprochée.
        let mut codes: Numeros = BTreeMap::new();
        let mut q = conn.prepare_cached(
            "SELECT COALESCE(sp.set_code, ''), sp.rarity \
             FROM set_prints sp JOIN set_locales sl ON sl.id = sp.set_locale_id \
             WHERE sl.set_uuid = ?1 AND sl.language = ?2 AND sp.card_uuid = ?3 \
               AND COALESCE(sp.rarity, '') <> ''",
        )?;
        let lignes = q.query_map(params![ext.set_uuid, ext.langue, card_uuid], |l| {
            Ok((l.get::<_, String>(0)?, l.get::<_, String>(1)?))
        })?;
        for l in lignes {
            let (code, rarete) = l?;
            codes.entry(code).or_default().insert(rarete);
        }
        let nom = nom_base.as_deref().unwrap_or(d.nom);
        let connues = Connues {
            base: codes,
            classeur: d.raretes.to_vec(),
        };
        Ok(decider(
            ext,
            versions,
            &connues,
            d.rarete,
            nom,
            &self.index.prix,
            prix_du,
        ))
    }
}

/// La décision, une fois la carte trouvée — fonction pure.
#[must_use]
pub fn decider(
    ext: &Extension,
    versions: &Versions,
    connues: &Connues,
    rarete: &str,
    nom: &str,
    prix: &HashMap<i64, Prix>,
    prix_du: String,
) -> Offre {
    let tous: Vec<i64> = versions
        .ids
        .iter()
        .chain(&versions.ajouts)
        .copied()
        .collect();
    let fourchette = fourchette(&tous, prix);
    let base: BTreeSet<String> = connues.base.values().flatten().cloned().collect();
    let classeur: BTreeSet<String> = connues
        .classeur
        .iter()
        .filter(|r| !r.trim().is_empty())
        .cloned()
        .collect();
    let offre = |cible: Cible, version: Option<String>, prix: PrixOffre, raison: Raison| Offre {
        cible,
        version,
        prix,
        raison,
        extension: ext.libelle.clone(),
        prix_du: prix_du.clone(),
    };
    let recherche = Cible::Recherche {
        nom: nom.to_owned(),
        id_extension: Some(ext.id),
    };
    let exact = |id: i64| {
        prix.get(&id)
            .map_or(PrixOffre::Aucun, |p| PrixOffre::Exact(*p))
    };

    let n = versions.ids.len();
    // Le classeur d'abord, la base ensuite (cf. `Demande::raretes`).
    let principale = if classeur.is_empty() {
        &base
    } else {
        &classeur
    };
    let un_seul_numero = !classeur.is_empty() || connues.base.len() <= 1;
    if n == 1 && principale.len() <= 1 && un_seul_numero {
        if let Some(id) = versions.ids.first() {
            return offre(Cible::Produit(*id), None, exact(*id), Raison::VersionUnique);
        }
    }
    // L'ordre complet des versions de la carte chez Cardmarket.
    let retenues = [&classeur, &base]
        .into_iter()
        .find(|s| !s.is_empty() && s.len() == n);
    let ordre: Vec<String> = if let Some(retenues) = retenues {
        let raretes: Vec<String> = retenues.iter().cloned().collect();
        let Some(ordre) = regles::ordonner(&raretes, &ext.contexte()) else {
            let meme_rarete = raretes
                .iter()
                .any(|r| regles::decouper_cle(r).1 != Distinction::Aucune);
            let raison = if meme_rarete {
                Raison::PlusieursVersionsDeLaRarete
            } else {
                Raison::OrdreDesVersionsInconnu
            };
            return offre(recherche, None, fourchette, raison);
        };
        ordre
    } else if let Some(sequence) = regles::ordres_confirmes(ext.id).iter().find(|s| {
        s.len() == n && !principale.is_empty() && principale.iter().all(|r| s.contains(r))
    }) {
        // Cardmarket a des versions que nous ne nommons pas — une Ultra
        // Overframe que la base ignore, une alternative que le classeur ne
        // distingue pas de son originale. Un ordre relevé de la même longueur,
        // qui contient toutes les nôtres, les place.
        //
        // Sauf la version qu'une de ces inconnues pourrait être : la ligne
        // « Quarter Century » de RA04 « Cyber Dragon Infinity », dont les deux
        // fichiers sont des `-AA`, peut être l'originale ou l'alternative.
        let (rarete_de_la_ligne, _) = regles::decouper_cle(rarete);
        let ambigue = sequence.iter().any(|c| {
            c != rarete
                && !principale.contains(c)
                && regles::decouper_cle(c).0 == rarete_de_la_ligne
        });
        if ambigue {
            return offre(
                recherche,
                None,
                fourchette,
                Raison::NombreDeVersions {
                    cardmarket: tous.len(),
                    base: principale.len(),
                },
            );
        }
        sequence.clone()
    } else {
        let raison = if un_seul_numero {
            Raison::NombreDeVersions {
                cardmarket: tous.len(),
                base: principale.len(),
            }
        } else {
            Raison::PlusieursNumeros
        };
        return offre(recherche, None, fourchette, raison);
    };
    let Some(k) = ordre.iter().position(|r| r == rarete) else {
        return offre(recherche, None, fourchette, Raison::RareteInconnue);
    };
    // Des ajouts tardifs décalent les numéros : sans eux seulement, le rang
    // de la rareté est le numéro affiché par Cardmarket.
    let version = versions.ajouts.is_empty().then(|| {
        format!(
            "V.{} · {}",
            k + 1,
            regles::libelle_de_version(rarete, ext.region, ext.id)
        )
    });
    // Deux illustrations d'une même rareté ont la même valeur : les prix ne
    // vérifient pas leur attribution. Là où le set les a créées à part (RA04 :
    // l'alternative de « El Shaddoll Winda » quatre jours avant les autres
    // versions), le rang V.n est sûr, le produit non — pour toute la carte.
    let a_des_illustrations = ordre
        .iter()
        .any(|c| matches!(regles::decouper_cle(c).1, Distinction::Illustration(_)));
    if a_des_illustrations && regles::illustrations_saisies_a_part(ext.id) {
        return offre(
            recherche,
            version,
            fourchette,
            Raison::IllustrationsSaisiesAPart,
        );
    }
    // L'identifiant de chaque version, selon le sens de saisie du set.
    let attribution: Option<(Vec<i64>, Raison)> = match ext.sens {
        Sens::DansLOrdre => Some((versions.ids.clone(), Raison::Ordonnee)),
        Sens::Inverse => Some((
            versions.ids.iter().rev().copied().collect(),
            Raison::Ordonnee,
        )),
        Sens::Inconnu if ext.region == Region::Tcg => {
            Some((versions.ids.clone(), Raison::OrdreDeSaisieSuppose))
        }
        Sens::Inconnu | Sens::Melange => None,
    };
    let Some((ids, raison)) = attribution else {
        return offre(recherche, version, fourchette, Raison::SaisieMelangee);
    };
    // Un set saisi dans l'ordre peut avoir des cartes qui ne le sont pas :
    // une version ajoutée après coup au milieu (PHRE « Kewl Tune Clip »), une
    // version créée à part avant les autres (BPRO « Topologic Blaster
    // Dragon »). Les prix de la carte doivent coller à l'attribution.
    if !vraisemblable(&ordre, &ids, prix) {
        return offre(recherche, version, fourchette, Raison::SaisieDouteuse);
    }
    match ids.get(k) {
        Some(id) => offre(Cible::Produit(*id), version, exact(*id), raison),
        None => offre(recherche, version, fourchette, Raison::SaisieMelangee),
    }
}

/// Écart de prix en deçà duquel deux versions ne témoignent de rien.
pub const ECART_DE_PRIX_SIGNIFICATIF: u32 = 10;

/// Les prix d'une carte collent-ils à une attribution des identifiants ?
///
/// Pour chaque paire de raretés d'au moins un cran d'écart en valeur, la
/// plus précieuse ne doit pas coûter nettement moins que l'autre : au moins
/// 70 % de son prix, à dix centimes près. Une paire sans prix ne témoigne
/// pas. L'erreur est volontairement du côté prudent : une attribution juste
/// mais aux prix étranges perd son lien direct (la recherche reste), une
/// attribution fausse ne passe pas.
///
/// ```
/// use std::collections::HashMap;
/// use ygo_app::cardmarket::{vraisemblable, Prix};
/// let p = |t: u32| Prix { tendance: Some(t), ..Prix::default() };
/// let prix = HashMap::from([(1, p(190)), (2, p(795)), (3, p(2494))]);
/// let ordre = ["Secret Rare", "Prismatic Secret Rare", "Ultimate Rare"].map(String::from);
/// // BPRO « Topologic Blaster Dragon » : l'Ultimate, créée la première, a le
/// // plus petit identifiant. L'ordre des identifiants ferait de la Prismatic
/// // (24,94 €) une Ultimate.
/// assert!(!vraisemblable(&ordre, &[1, 2, 3], &prix));
/// assert!(vraisemblable(&ordre, &[2, 3, 1], &prix));
/// ```
#[must_use]
pub fn vraisemblable(ordre: &[String], ids: &[i64], prix: &HashMap<i64, Prix>) -> bool {
    let valeur_et_prix: Vec<(Option<f64>, Option<u32>)> = ordre
        .iter()
        .zip(ids)
        .map(|(r, id)| (regles::valeur(r), prix.get(id).and_then(Prix::reference)))
        .collect();
    for (i, (va, pa)) in valeur_et_prix.iter().enumerate() {
        for (vb, pb) in valeur_et_prix.iter().skip(i + 1) {
            let (Some(va), Some(vb), Some(pa), Some(pb)) = (va, vb, pa, pb) else {
                continue;
            };
            // `haut` est la plus précieuse des deux.
            let (haut, bas) = if vb - va >= ECART_DE_VRAISEMBLANCE {
                (*pb, *pa)
            } else if va - vb >= ECART_DE_VRAISEMBLANCE {
                (*pa, *pb)
            } else {
                continue;
            };
            if u64::from(haut) * 10 + u64::from(ECART_DE_PRIX_SIGNIFICATIF) * 10
                < u64::from(bas) * 7
            {
                return false;
            }
        }
    }
    true
}

/// Écart de valeur à partir duquel une paire sert au contrôle de
/// vraisemblance — plus fin que pour le sens de saisie : Prismatic contre
/// Ultimate (1,3) doit compter.
pub const ECART_DE_VRAISEMBLANCE: f64 = 1.0;

/// La fourchette des prix de référence d'un ensemble de produits.
fn fourchette(ids: &[i64], prix: &HashMap<i64, Prix>) -> PrixOffre {
    let valeurs: Vec<u32> = ids
        .iter()
        .filter_map(|id| prix.get(id).and_then(Prix::reference))
        .collect();
    match (valeurs.iter().min(), valeurs.iter().max()) {
        (Some(min), Some(max)) => PrixOffre::Fourchette {
            min: *min,
            max: *max,
        },
        _ => PrixOffre::Aucun,
    }
}

/// Un instantané de l'installation : combien de lignes de classeur ont un
/// lien direct, une recherche, ou rien — pour `ygo-cli cardmarket`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Couverture {
    /// Lignes lues.
    pub lignes: usize,
    /// Lien direct, produit sûr.
    pub directes_sures: usize,
    /// Lien direct, sens de saisie supposé.
    pub directes_supposees: usize,
    /// Recherche avec le rang V.n à ouvrir.
    pub recherches_avec_version: usize,
    /// Recherche seule.
    pub recherches: usize,
    /// Par raison.
    pub raisons: BTreeMap<String, usize>,
}

impl Couverture {
    /// Compte une offre.
    pub fn compter(&mut self, offre: &Offre) {
        self.lignes += 1;
        match (offre.est_directe(), offre.raison) {
            (true, Raison::OrdreDeSaisieSuppose) => self.directes_supposees += 1,
            (true, _) => self.directes_sures += 1,
            (false, _) if offre.version.is_some() => self.recherches_avec_version += 1,
            (false, _) => self.recherches += 1,
        }
        *self
            .raisons
            .entry(
                format!("{:?}", offre.raison)
                    .split(' ')
                    .next()
                    .unwrap_or("")
                    .to_owned(),
            )
            .or_default() += 1;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Les lignes d'un classeur, et ce que valent ses cartes
// ─────────────────────────────────────────────────────────────────────────────

/// Une ligne de classeur et son offre.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OffreDeLigne {
    /// `rowid` de la ligne.
    pub rowid: i64,
    /// Numéro du tirage.
    pub set_code: String,
    /// Sa version : la rareté, ou sa clé de version (cf. [`cles_de_version`]).
    pub version: String,
    /// Nom, tel que le classeur l'écrit.
    pub nom: String,
    /// Exemplaires possédés.
    pub quantite: i64,
    /// L'offre — celle que montre la fiche de la ligne.
    pub offre: Offre,
}

/// L'offre de chaque ligne d'un classeur, dans l'ordre du classeur, calculée
/// comme la fiche la calcule ([`crate::fiche::lire`]) : mêmes versions du
/// numéro, même carte.
///
/// Avec `possedees_seules`, les lignes sans exemplaire sont sautées ; leurs
/// versions servent quand même à nommer celles des autres (une Ultra Rare
/// possédée reste « Ultra Rare » face à une Overframe qui ne l'est pas).
///
/// # Errors
///
/// Si le classeur ou `cardinfo.db` ne se lisent pas.
pub fn offres_du_classeur(
    index: &IndexCharge,
    cardinfo: &Connection,
    classeur: &Connection,
    possedees_seules: bool,
) -> Result<Vec<OffreDeLigne>> {
    // Les classeurs anciens n'ont pas toutes les colonnes : une colonne
    // absente ferait échouer toute la requête.
    let colonnes = ygo_db::migrations::colonnes(classeur, "cards")?;
    let colonne = |nom: &str, defaut: &str| {
        if colonnes.iter().any(|c| c == nom) {
            format!("COALESCE({nom}, {defaut})")
        } else {
            defaut.to_owned()
        }
    };
    let sql = format!(
        "SELECT rowid, COALESCE(set_code, ''), COALESCE(rarity, ''), COALESCE(name, ''), \
                {}, COALESCE(quantite, 0), {}, {}, {}, {} \
         FROM cards ORDER BY sort_order, rowid",
        colonne("card_uuid", "''"),
        colonne("card_image_uuid", "''"),
        colonne("card_image_url", "''"),
        colonne("extended_art", "0"),
        colonne("card_image_id", "0"),
    );
    /// Ce qu'il faut d'une ligne pour sa demande.
    struct Brute {
        set_code: String,
        nom: String,
        uuid: String,
        quantite: i64,
        image_id: i64,
        version: LigneDeClasseur,
    }
    let mut q = classeur.prepare(&sql)?;
    let lignes = q
        .query_map([], |l| {
            Ok(Brute {
                set_code: l.get(1)?,
                nom: l.get(3)?,
                uuid: l.get(4)?,
                quantite: l.get::<_, i64>(5)?.max(0),
                image_id: l.get(9)?,
                version: LigneDeClasseur {
                    rowid: l.get(0)?,
                    rarete: l.get(2)?,
                    fichier: fichier_de_ligne(&l.get::<_, String>(6)?, &l.get::<_, String>(7)?),
                    overframe: l.get::<_, i64>(8)? != 0,
                },
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    // Les versions de chaque numéro, comme `lignes_du_numero` les lit : les
    // lignes qui ont une rareté, dans l'ordre du classeur.
    let mut par_numero: HashMap<&str, Vec<LigneDeClasseur>> = HashMap::new();
    for l in lignes.iter().filter(|l| !l.version.rarete.is_empty()) {
        par_numero
            .entry(l.set_code.as_str())
            .or_default()
            .push(l.version.clone());
    }
    let mut cle_de_ligne: HashMap<i64, String> = HashMap::new();
    let mut versions_du_numero: HashMap<&str, Vec<String>> = HashMap::new();
    for (set_code, du_numero) in &par_numero {
        let mut versions: Vec<String> = Vec::new();
        for (l, cle) in du_numero.iter().zip(cles_de_version(set_code, du_numero)) {
            if !versions.contains(&cle) {
                versions.push(cle.clone());
            }
            cle_de_ligne.insert(l.rowid, cle);
        }
        versions_du_numero.insert(set_code, versions);
    }

    let mut offres = Vec::new();
    for l in &lignes {
        if possedees_seules && l.quantite <= 0 {
            continue;
        }
        let version = cle_de_ligne
            .get(&l.version.rowid)
            .cloned()
            .unwrap_or_else(|| l.version.rarete.clone());
        let (uuid, _) = crate::fiche::resoudre(
            cardinfo,
            &l.uuid,
            Some(l.image_id).filter(|id| *id != 0),
            &l.set_code,
        )?;
        let demande = Demande {
            set_code: &l.set_code,
            rarete: &version,
            nom: &l.nom,
            card_uuid: uuid.as_deref(),
            raretes: versions_du_numero
                .get(l.set_code.as_str())
                .map_or(&[], Vec::as_slice),
        };
        let offre = index.offre(cardinfo, &demande)?;
        offres.push(OffreDeLigne {
            rowid: l.version.rowid,
            set_code: l.set_code.clone(),
            version,
            nom: l.nom.clone(),
            quantite: l.quantite,
            offre,
        });
    }
    Ok(offres)
}

/// Écart, en centimes, en deçà duquel une moyenne n'est jamais tenue pour
/// aberrante : 0,10 € contre 0,02 €, c'est le bruit des cartes communes.
pub const ECART_ABERRANT: u32 = 100;

impl Prix {
    /// Le prix d'**un exemplaire**, pour chiffrer une collection : la moyenne
    /// des ventes sur trente jours.
    ///
    /// Sauf quand elle s'écarte nettement — plus du triple, à un euro près —
    /// de la tendance **et** du plus bas : une vente hors norme dans le mois
    /// (1 % des produits, octobre 2026 : 412,49 € de moyenne pour 34,19 € de
    /// tendance et 4,56 € au plus bas). Le médian des trois
    /// ([`Prix::reference`]) la remplace alors, comme quand elle manque.
    ///
    /// Sans vente ni tendance, rien : un plus bas seul n'est qu'une offre, pas
    /// une valeur (LOCR-JP001 Grand Master Rare, jamais vendue : une seule
    /// annonce, à 25 000 €).
    ///
    /// ```
    /// use ygo_app::cardmarket::Prix;
    /// let p = |t, b, m| Prix { tendance: t, bas: b, moyenne30: m };
    /// assert_eq!(p(Some(33), Some(7), Some(41)).unitaire(), Some(41));
    /// assert_eq!(p(Some(3_419), Some(456), Some(41_249)).unitaire(), Some(3_419));
    /// assert_eq!(p(Some(500), Some(400), Some(90)).unitaire(), Some(400));
    /// assert_eq!(p(Some(10), Some(2), Some(80)).unitaire(), Some(80));
    /// assert_eq!(p(Some(100), Some(51), None).unitaire(), Some(75));
    /// assert_eq!(p(None, Some(2_500_000), None).unitaire(), None);
    /// assert_eq!(p(None, None, None).unitaire(), None);
    /// ```
    #[must_use]
    pub fn unitaire(&self) -> Option<u32> {
        let Some(moyenne) = self.moyenne30 else {
            self.tendance?;
            return self.reference();
        };
        let (Some(t), Some(b)) = (self.tendance, self.bas) else {
            return Some(moyenne);
        };
        let (bas, haut) = (t.min(b), t.max(b));
        let au_dessus = moyenne > haut.saturating_mul(3) && moyenne - haut > ECART_ABERRANT;
        let au_dessous = moyenne.saturating_mul(3) < bas && bas - moyenne > ECART_ABERRANT;
        if au_dessus || au_dessous {
            self.reference()
        } else {
            Some(moyenne)
        }
    }
}

impl Offre {
    /// Le prix d'un exemplaire de la ligne ([`Prix::unitaire`]) — `None`
    /// quand le produit n'est pas sûr : une fourchette chiffrerait peut-être
    /// une autre version, une Starlight au prix d'une Super.
    #[must_use]
    pub fn prix_unitaire(&self) -> Option<u32> {
        match self.prix {
            PrixOffre::Exact(p) => p.unitaire(),
            PrixOffre::Fourchette { .. } | PrixOffre::Aucun => None,
        }
    }
}

/// Ce que valent des exemplaires, selon les prix Cardmarket.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Valeur {
    /// La somme, en centimes.
    pub centimes: u64,
    /// Les exemplaires chiffrés.
    pub chiffres: u64,
    /// Les exemplaires qu'on ne chiffre pas : version incertaine, carte non
    /// retrouvée chez Cardmarket, ou pas de prix publié.
    pub non_chiffres: u64,
}

impl Valeur {
    /// Compte les exemplaires d'une ligne, au prix d'un exemplaire s'il est
    /// sûr. Une quantité nulle ne compte pas.
    ///
    /// ```
    /// use ygo_app::cardmarket::Valeur;
    /// let mut v = Valeur::default();
    /// v.ajouter(3, Some(150));
    /// v.ajouter(2, None);
    /// v.ajouter(0, Some(9_999));
    /// assert_eq!(v, Valeur { centimes: 450, chiffres: 3, non_chiffres: 2 });
    /// ```
    pub fn ajouter(&mut self, quantite: i64, prix: Option<u32>) {
        let Ok(exemplaires) = u64::try_from(quantite) else {
            return;
        };
        if exemplaires == 0 {
            return;
        }
        match prix {
            Some(p) => {
                self.centimes = self
                    .centimes
                    .saturating_add(exemplaires.saturating_mul(u64::from(p)));
                self.chiffres = self.chiffres.saturating_add(exemplaires);
            }
            None => self.non_chiffres = self.non_chiffres.saturating_add(exemplaires),
        }
    }

    /// Ajoute une autre valeur — celle d'un autre classeur.
    pub fn cumuler(&mut self, autre: Self) {
        self.centimes = self.centimes.saturating_add(autre.centimes);
        self.chiffres = self.chiffres.saturating_add(autre.chiffres);
        self.non_chiffres = self.non_chiffres.saturating_add(autre.non_chiffres);
    }

    /// Les exemplaires comptés, chiffrés ou non.
    #[must_use]
    pub fn exemplaires(&self) -> u64 {
        self.chiffres.saturating_add(self.non_chiffres)
    }
}

/// La valeur des possédées de chaque classeur.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Valorisation {
    /// Par code de classeur.
    pub classeurs: BTreeMap<String, Valeur>,
    /// Le prix d'un exemplaire de chaque ligne possédée, par classeur puis
    /// par `rowid` — `None` quand la version n'est pas sûre. C'est la
    /// matière des colonnes de l'inventaire (2026-10-23).
    pub lignes: HashMap<String, HashMap<i64, Option<u32>>>,
    /// Date du guide des prix.
    pub prix_du: String,
}

impl Valorisation {
    /// Le prix d'un exemplaire d'une ligne possédée — `None` si sa version
    /// n'est pas sûre, ou si la ligne n'a pas été chiffrée (possédée
    /// depuis).
    ///
    /// ```
    /// use std::collections::HashMap;
    /// use ygo_app::cardmarket::Valorisation;
    /// let v = Valorisation {
    ///     lignes: HashMap::from([("RA05".to_owned(), HashMap::from([(1, Some(388)), (2, None)]))]),
    ///     ..Valorisation::default()
    /// };
    /// assert_eq!(v.prix("RA05", 1), Some(388));
    /// assert_eq!(v.prix("RA05", 2), None);
    /// assert_eq!(v.prix("RA05", 3), None);
    /// assert_eq!(v.prix("LOCR-JP", 1), None);
    /// assert!(v.chiffree("RA05", 2) && !v.chiffree("RA05", 3));
    /// ```
    #[must_use]
    pub fn prix(&self, classeur: &str, rowid: i64) -> Option<u32> {
        self.lignes.get(classeur)?.get(&rowid).copied().flatten()
    }

    /// La ligne a-t-elle été chiffrée — avec ou sans prix sûr ? Une ligne
    /// possédée depuis le chiffrage ne l'est pas : il est à refaire.
    #[must_use]
    pub fn chiffree(&self, classeur: &str, rowid: i64) -> bool {
        self.lignes
            .get(classeur)
            .is_some_and(|l| l.contains_key(&rowid))
    }

    /// La valeur de tous les classeurs.
    #[must_use]
    pub fn total(&self) -> Valeur {
        let mut total = Valeur::default();
        for v in self.classeurs.values() {
            total.cumuler(*v);
        }
        total
    }
}

/// Chiffre les cartes possédées des classeurs donnés — `None` tant qu'il n'y
/// a pas d'index Cardmarket ou de base des cartes.
///
/// Un dixième de seconde pour les dix-sept classeurs de l'utilisateur (1 325
/// lignes possédées, octobre 2026), plusieurs secondes sur un disque froid :
/// à faire hors du fil de l'interface. Un classeur illisible est sauté.
///
/// # Errors
///
/// Si `cardinfo.db` ne s'ouvre pas.
pub fn valoriser(paths: &Paths, codes: &[String]) -> Result<Option<Valorisation>> {
    let Some(index) = charge(paths) else {
        return Ok(None);
    };
    if !paths.cardinfo_db().is_file() {
        return Ok(None);
    }
    let cardinfo = ygo_db::connexion::ouvrir_lecture_seule(paths.cardinfo_db())?;
    let mut classeurs = BTreeMap::new();
    let mut lignes = HashMap::new();
    for code in codes {
        let valeur = ygo_db::connexion::ouvrir_lecture_seule(paths.classeur_db(code))
            .map_err(crate::error::AppError::from)
            .and_then(|conn| offres_du_classeur(&index, &cardinfo, &conn, true));
        match valeur {
            Ok(offres) => {
                let mut v = Valeur::default();
                let mut prix = HashMap::with_capacity(offres.len());
                for l in &offres {
                    let unitaire = l.offre.prix_unitaire();
                    v.ajouter(l.quantite, unitaire);
                    prix.insert(l.rowid, unitaire);
                }
                classeurs.insert(code.clone(), v);
                lignes.insert(code.clone(), prix);
            }
            Err(e) => {
                tracing::warn!(classeur = %code, erreur = %e, "Cardmarket : classeur non chiffré")
            }
        }
    }
    Ok(Some(Valorisation {
        classeurs,
        lignes,
        prix_du: index.index.empreinte.prix.clone(),
    }))
}

/// Le prix d'un exemplaire de chaque ligne d'un classeur : la valeur d'un
/// classeur ouvert se refait ainsi à chaque quantité changée, sans relire la
/// base.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrixDesLignes {
    /// `rowid` → prix d'un exemplaire, `None` quand il n'est pas sûr.
    pub lignes: HashMap<i64, Option<u32>>,
    /// Date du guide des prix.
    pub prix_du: String,
}

impl PrixDesLignes {
    /// La valeur de lignes, d'après leurs quantités : `(rowid, quantité)`. Une
    /// ligne inconnue — ajoutée depuis — n'est pas chiffrée.
    ///
    /// ```
    /// use std::collections::HashMap;
    /// use ygo_app::cardmarket::{PrixDesLignes, Valeur};
    /// let prix = PrixDesLignes {
    ///     lignes: HashMap::from([(1, Some(250)), (2, None)]),
    ///     prix_du: String::new(),
    /// };
    /// assert_eq!(
    ///     prix.valeur([(1, 2), (2, 1), (3, 4), (4, 0)]),
    ///     Valeur { centimes: 500, chiffres: 2, non_chiffres: 5 }
    /// );
    /// ```
    #[must_use]
    pub fn valeur(&self, lignes: impl IntoIterator<Item = (i64, i64)>) -> Valeur {
        let mut v = Valeur::default();
        for (rowid, quantite) in lignes {
            v.ajouter(quantite, self.lignes.get(&rowid).copied().flatten());
        }
        v
    }
}

/// Les prix des lignes d'un classeur — `None` tant qu'il n'y a pas d'index
/// Cardmarket ou de base des cartes.
///
/// # Errors
///
/// Si le classeur ou `cardinfo.db` ne se lisent pas.
pub fn prix_des_lignes(paths: &Paths, code: &str) -> Result<Option<PrixDesLignes>> {
    let Some(index) = charge(paths) else {
        return Ok(None);
    };
    if !paths.cardinfo_db().is_file() {
        return Ok(None);
    }
    let cardinfo = ygo_db::connexion::ouvrir_lecture_seule(paths.cardinfo_db())?;
    let conn = ygo_db::connexion::ouvrir_lecture_seule(paths.classeur_db(code))?;
    let lignes = offres_du_classeur(&index, &cardinfo, &conn, false)?
        .into_iter()
        .map(|l| (l.rowid, l.offre.prix_unitaire()))
        .collect();
    Ok(Some(PrixDesLignes {
        lignes,
        prix_du: index.index.empreinte.prix.clone(),
    }))
}

/// La date du fichier d'index — pour savoir, sans le relire, s'il a changé
/// depuis un chiffrage.
#[must_use]
pub fn date_de_l_index(paths: &Paths) -> Option<SystemTime> {
    std::fs::metadata(chemin_index(paths))
        .and_then(|m| m.modified())
        .ok()
}

/// Lit le chemin d'un fichier Cardmarket d'une installation.
#[must_use]
pub fn chemin_fichier(paths: &Paths, fichier: Fichier) -> PathBuf {
    fichiers::chemin(&dossier(paths), fichier)
}

/// Le dossier existe-t-il et contient-il le catalogue ?
#[must_use]
pub fn a_des_donnees(paths: &Paths) -> bool {
    chemin_fichier(paths, Fichier::Cartes).is_file()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]

    use super::*;
    use ygo_sources::cardmarket::Produit;

    fn produit(id: i64, nom: &str, ext: i64, date: &str) -> Produit {
        Produit {
            id,
            name: nom.to_owned(),
            id_extension: ext,
            ajoute_le: date.to_owned(),
        }
    }

    fn prix_publie(id: i64, trend: f64) -> PrixPublie {
        PrixPublie {
            id,
            trend: Some(trend),
            low: Some(trend / 2.0),
            avg: None,
            avg1: None,
            avg7: None,
            avg30: None,
        }
    }

    /// Une base de deux locales : un set TCG (« EN ») et un set OCG (« JP »).
    fn base() -> Base {
        let mut b = Base::default();
        b.locales.insert(
            1,
            Locale {
                set_uuid: "set-a".into(),
                langue: "en".into(),
                prefixe: "AAA-EN".into(),
                sortie: Some("2024-05-01".into()),
            },
        );
        b.locales.insert(
            2,
            Locale {
                set_uuid: "set-a".into(),
                langue: "jp".into(),
                prefixe: "AAA-JP".into(),
                sortie: Some("2024-04-01".into()),
            },
        );
        for (sl, nom, code, raretes) in [
            (1, "alpha", "AAA-EN001", vec!["Ultra Rare", "Secret Rare"]),
            (1, "beta", "AAA-EN002", vec!["Common"]),
            (
                1,
                "gamma",
                "AAA-EN003",
                vec!["Super Rare", "Starlight Rare"],
            ),
            (1, "delta", "AAA-EN004", vec!["Common", "Ultra Rare"]),
            (1, "epsilon", "AAA-EN005", vec!["Rare", "Secret Rare"]),
            (1, "zeta", "AAA-EN006", vec!["Common", "Secret Rare"]),
            (2, "alpha", "AAA-JP001", vec!["Ultra Rare"]),
            (2, "beta", "AAA-JP002", vec!["Common"]),
            (2, "gamma", "AAA-JP003", vec!["Super Rare"]),
        ] {
            let e = b
                .tirages
                .entry(sl)
                .or_default()
                .entry(nom.to_owned())
                .or_default()
                .entry(code.to_owned())
                .or_default();
            for r in raretes {
                e.insert(r.to_owned());
            }
        }
        b
    }

    fn connues(base: Numeros) -> Connues {
        Connues {
            base,
            classeur: Vec::new(),
        }
    }

    fn catalogue(produits: Vec<Produit>) -> Catalogue {
        Catalogue {
            cree_le: "2026-10-20".into(),
            products: produits,
        }
    }

    /// Extension 10 : TCG, saisie dans l'ordre ; extension 20 : OCG.
    fn index_de_test() -> Index {
        let d = "2024-05-02 10:00:00";
        let cartes = catalogue(vec![
            produit(100, "Alpha", 10, d),
            produit(101, "Alpha", 10, d),
            produit(102, "Beta", 10, d),
            produit(103, "Gamma", 10, d),
            produit(104, "Gamma", 10, d),
            produit(105, "Delta", 10, d),
            produit(106, "Delta", 10, d),
            produit(107, "Epsilon", 10, d),
            produit(108, "Epsilon", 10, d),
            produit(109, "Zeta", 10, d),
            produit(110, "Zeta", 10, d),
            // Une réédition tardive de Beta.
            produit(111, "Beta", 10, "2025-09-01 10:00:00"),
            produit(200, "Alpha", 20, d),
            produit(201, "Beta", 20, d),
            produit(202, "Gamma", 20, d),
        ]);
        let scelles = catalogue(vec![
            produit(900, "Set A Booster Box", 10, d),
            produit(901, "Set A (OCG) Booster Box", 20, d),
        ]);
        let guide = GuidePrix {
            cree_le: "2026-10-20T02:44:58+0200".into(),
            prix: vec![
                prix_publie(100, 1.0),
                prix_publie(101, 9.0),
                prix_publie(102, 0.1),
                prix_publie(103, 0.5),
                prix_publie(104, 30.0),
                prix_publie(105, 0.05),
                prix_publie(106, 2.0),
                prix_publie(107, 0.2),
                prix_publie(108, 6.0),
                prix_publie(109, 0.1),
                prix_publie(110, 4.0),
                prix_publie(111, 0.3),
                prix_publie(200, 1.0),
            ],
        };
        assembler(&cartes, &scelles, &guide, &base(), Empreinte::default())
    }

    #[test]
    fn les_extensions_se_rapprochent_de_la_bonne_region() {
        let index = index_de_test();
        assert_eq!(index.extensions.len(), 2);
        let tcg = index.extensions.iter().find(|e| e.id == 10).unwrap();
        assert_eq!(tcg.region, Region::Tcg);
        assert_eq!(tcg.prefixe, "AAA-EN");
        assert_eq!(tcg.libelle.as_deref(), Some("Set A"));
        let ocg = index.extensions.iter().find(|e| e.id == 20).unwrap();
        assert_eq!(ocg.region, Region::Ocg);
        assert_eq!(ocg.prefixe, "AAA-JP");
        assert_eq!(ocg.libelle.as_deref(), Some("Set A (OCG)"));
    }

    #[test]
    fn le_sens_de_saisie_se_mesure_aux_prix() {
        let index = index_de_test();
        let tcg = index.extensions.iter().find(|e| e.id == 10).unwrap();
        // Gamma (Super/Starlight), Delta (Common/Ultra), Epsilon (Rare/Secret),
        // Zeta (Common/Secret) : la version précieuse a l'identifiant le plus
        // grand. Alpha (Ultra/Secret) est trop serré pour témoigner.
        assert_eq!(tcg.temoins, (4, 0));
        assert_eq!(tcg.sens, Sens::DansLOrdre);
        let beta = &tcg.cartes["beta"];
        assert_eq!(beta.ids, vec![102]);
        assert_eq!(beta.ajouts, vec![111]);
    }

    #[test]
    fn decision_directe_quand_le_produit_est_sur() {
        let index = IndexCharge::new(index_de_test());
        let ext = index.extensions_du_set("set-a", Region::Tcg)[0];
        let codes = |r: &[&str]| {
            BTreeMap::from([(
                "AAA-EN001".to_owned(),
                r.iter().map(|x| (*x).to_owned()).collect::<BTreeSet<_>>(),
            )])
        };
        let o = decider(
            ext,
            &ext.cartes["alpha"],
            &connues(codes(&["Ultra Rare", "Secret Rare"])),
            "Secret Rare",
            "Alpha",
            &index.index.prix,
            String::new(),
        );
        assert_eq!(o.cible, Cible::Produit(101));
        assert_eq!(o.raison, Raison::Ordonnee);
        assert_eq!(o.version.as_deref(), Some("V.2 · Secret Rare"));
        assert!(matches!(o.prix, PrixOffre::Exact(p) if p.tendance == Some(900)));
    }

    #[test]
    fn un_ajout_tardif_fait_chercher_plutot_que_deviner() {
        let index = IndexCharge::new(index_de_test());
        let ext = index.extensions_du_set("set-a", Region::Tcg)[0];
        let codes = BTreeMap::from([(
            "AAA-EN002".to_owned(),
            BTreeSet::from(["Common".to_owned()]),
        )]);
        // Une version d'origine, une réédition : la base n'en connaît qu'une,
        // c'est celle d'origine.
        let o = decider(
            ext,
            &ext.cartes["beta"],
            &connues(codes.clone()),
            "Common",
            "Beta",
            &index.index.prix,
            String::new(),
        );
        assert_eq!(o.cible, Cible::Produit(102));
        assert_eq!(o.raison, Raison::VersionUnique);
    }

    #[test]
    fn saisie_inverse_et_saisie_melangee() {
        let mut ext = index_de_test()
            .extensions
            .into_iter()
            .find(|e| e.id == 10)
            .unwrap();
        let prix = HashMap::from([
            (
                100,
                Prix {
                    tendance: Some(100),
                    ..Prix::default()
                },
            ),
            (
                101,
                Prix {
                    tendance: Some(900),
                    ..Prix::default()
                },
            ),
        ]);
        let codes = BTreeMap::from([(
            "AAA-EN001".to_owned(),
            BTreeSet::from(["Ultra Rare".to_owned(), "Secret Rare".to_owned()]),
        )]);
        // À l'envers : la V.2 est la première créée — et c'est elle qui vaut
        // le plus cher, sinon l'attribution ne serait pas vraisemblable.
        let prix_a_l_envers = HashMap::from([
            (
                100,
                Prix {
                    tendance: Some(900),
                    ..Prix::default()
                },
            ),
            (
                101,
                Prix {
                    tendance: Some(100),
                    ..Prix::default()
                },
            ),
        ]);
        ext.sens = Sens::Inverse;
        let o = decider(
            &ext,
            &ext.cartes["alpha"],
            &connues(codes.clone()),
            "Secret Rare",
            "Alpha",
            &prix_a_l_envers,
            String::new(),
        );
        assert_eq!(
            o.cible,
            Cible::Produit(100),
            "à l'envers : la V.2 est le premier créé"
        );
        // Les mêmes prix lus dans l'ordre feraient de la Secret la moins
        // chère : douteux, on cherche.
        ext.sens = Sens::DansLOrdre;
        let o = decider(
            &ext,
            &ext.cartes["alpha"],
            &connues(codes.clone()),
            "Secret Rare",
            "Alpha",
            &prix_a_l_envers,
            String::new(),
        );
        assert_eq!(o.raison, Raison::SaisieDouteuse);
        assert!(!o.est_directe());

        ext.sens = Sens::Melange;
        let o = decider(
            &ext,
            &ext.cartes["alpha"],
            &connues(codes.clone()),
            "Secret Rare",
            "Alpha",
            &prix,
            String::new(),
        );
        assert!(!o.est_directe());
        assert_eq!(o.raison, Raison::SaisieMelangee);
        assert_eq!(o.version.as_deref(), Some("V.2 · Secret Rare"));
        assert_eq!(o.prix, PrixOffre::Fourchette { min: 100, max: 900 });
        assert_eq!(
            o.url("fr"),
            "https://www.cardmarket.com/fr/YuGiOh/Products/Search?searchString=Alpha&idExpansion=10"
        );

        // Une extension OCG au sens inconnu n'est pas supposée dans l'ordre.
        ext.sens = Sens::Inconnu;
        ext.region = Region::Ocg;
        let o = decider(
            &ext,
            &ext.cartes["alpha"],
            &connues(codes.clone()),
            "Secret Rare",
            "Alpha",
            &prix,
            String::new(),
        );
        assert!(!o.est_directe());
        ext.region = Region::Tcg;
        let o = decider(
            &ext,
            &ext.cartes["alpha"],
            &connues(codes.clone()),
            "Secret Rare",
            "Alpha",
            &prix,
            String::new(),
        );
        assert_eq!(o.raison, Raison::OrdreDeSaisieSuppose);
        assert_eq!(o.cible, Cible::Produit(101));
    }

    /// RA05 : la base n'a que des « Common » d'attente, le classeur a les
    /// vraies raretés — ce sont elles qui comptent.
    #[test]
    fn les_raretes_du_classeur_passent_devant_celles_de_la_base() {
        let index = IndexCharge::new(index_de_test());
        let ext = index.extensions_du_set("set-a", Region::Tcg)[0];
        let attente = BTreeMap::from([(
            "AAA-EN001".to_owned(),
            BTreeSet::from(["Common".to_owned()]),
        )]);
        let sans_classeur = decider(
            ext,
            &ext.cartes["alpha"],
            &connues(attente.clone()),
            "Secret Rare",
            "Alpha",
            &index.index.prix,
            String::new(),
        );
        assert!(matches!(
            sans_classeur.raison,
            Raison::NombreDeVersions {
                cardmarket: 2,
                base: 1
            }
        ));
        let avec_classeur = decider(
            ext,
            &ext.cartes["alpha"],
            &Connues {
                base: attente,
                classeur: vec!["Ultra Rare".to_owned(), "Secret Rare".to_owned()],
            },
            "Secret Rare",
            "Alpha",
            &index.index.prix,
            String::new(),
        );
        assert_eq!(avec_classeur.cible, Cible::Produit(101));
        assert_eq!(avec_classeur.version.as_deref(), Some("V.2 · Secret Rare"));
    }

    /// LOCH « Dark Magician, the Pharaoh's Servant » : cinq versions chez
    /// Cardmarket, trois dans la base ; l'ordre relevé place les nôtres.
    #[test]
    fn un_ordre_confirme_place_les_versions_que_la_base_ignore() {
        let mut ext = index_de_test()
            .extensions
            .into_iter()
            .find(|e| e.id == 10)
            .unwrap();
        ext.id = 6508; // LOCH, dont l'ordre à cinq versions est relevé
        ext.region = Region::Ocg;
        ext.sens = Sens::DansLOrdre;
        let versions = Versions {
            ids: vec![875_184, 875_185, 875_195, 875_196, 875_197],
            ajouts: Vec::new(),
        };
        let prix = HashMap::from([
            (
                875_184,
                Prix {
                    tendance: Some(181),
                    ..Prix::default()
                },
            ),
            (
                875_185,
                Prix {
                    tendance: Some(208),
                    ..Prix::default()
                },
            ),
            (
                875_195,
                Prix {
                    tendance: Some(4183),
                    ..Prix::default()
                },
            ),
            (
                875_196,
                Prix {
                    tendance: Some(15113),
                    ..Prix::default()
                },
            ),
            (
                875_197,
                Prix {
                    tendance: Some(599_999),
                    ..Prix::default()
                },
            ),
        ]);
        let connues = Connues {
            base: BTreeMap::new(),
            classeur: vec![
                "Ultra Rare".to_owned(),
                "Secret Rare".to_owned(),
                "Prismatic Secret Rare".to_owned(),
            ],
        };
        let o = decider(
            &ext,
            &versions,
            &connues,
            "Prismatic Secret Rare",
            "Dark Magician, the Pharaoh's Servant",
            &prix,
            String::new(),
        );
        assert_eq!(o.cible, Cible::Produit(875_196));
        assert_eq!(o.version.as_deref(), Some("V.4 · Secret Parallel Rare"));
        // L'Ultra ordinaire, ou l'Ultra Overframe que nous ne nommons pas ?
        // Sans le fichier de la ligne, on ne sait pas : on cherche.
        let o = decider(
            &ext,
            &versions,
            &connues,
            "Ultra Rare",
            "Dark Magician, the Pharaoh's Servant",
            &prix,
            String::new(),
        );
        assert!(!o.est_directe());
        assert_eq!(
            o.raison,
            Raison::NombreDeVersions {
                cardmarket: 5,
                base: 3
            }
        );
    }

    fn ext_de_test(id: i64, region: Region) -> Extension {
        let mut ext = index_de_test()
            .extensions
            .into_iter()
            .find(|e| e.id == 10)
            .unwrap();
        ext.id = id;
        ext.region = region;
        ext.sens = Sens::DansLOrdre;
        ext
    }

    fn tendances(v: &[(i64, u32)]) -> HashMap<i64, Prix> {
        v.iter()
            .map(|&(id, t)| {
                (
                    id,
                    Prix {
                        tendance: Some(t),
                        ..Prix::default()
                    },
                )
            })
            .collect()
    }

    fn cles(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_owned()).collect()
    }

    /// LOCH « Dark Magician, the Pharaoh's Servant » : la ligne Ultra Rare
    /// Overframe a sa page (V.3, « Ultra Parallel Rare » chez Cardmarket),
    /// distincte de celle de l'Ultra ordinaire (V.1).
    #[test]
    fn l_overframe_d_une_legende_a_sa_propre_page() {
        let ext = ext_de_test(6508, Region::Ocg);
        let versions = Versions {
            ids: vec![875_184, 875_185, 875_195, 875_196, 875_197],
            ajouts: Vec::new(),
        };
        let prix = tendances(&[
            (875_184, 181),
            (875_185, 208),
            (875_195, 4183),
            (875_196, 15113),
            (875_197, 599_999),
        ]);
        let connues = Connues {
            base: BTreeMap::new(),
            classeur: cles(&[
                "Prismatic Secret Rare",
                "Grand Master Rare",
                "Secret Rare",
                "Ultra Rare",
                "Ultra Rare (Overframe)",
            ]),
        };
        let nom = "Dark Magician, the Pharaoh's Servant";
        let o = decider(
            &ext,
            &versions,
            &connues,
            "Ultra Rare (Overframe)",
            nom,
            &prix,
            String::new(),
        );
        assert_eq!(o.cible, Cible::Produit(875_195));
        assert_eq!(o.raison, Raison::Ordonnee);
        assert_eq!(
            o.version.as_deref(),
            Some("V.3 · Ultra Parallel Rare (Overframe)")
        );
        let o = decider(
            &ext,
            &versions,
            &connues,
            "Ultra Rare",
            nom,
            &prix,
            String::new(),
        );
        assert_eq!(o.cible, Cible::Produit(875_184));
        assert_eq!(o.version.as_deref(), Some("V.1 · Ultra Rare"));
    }

    /// RA02 « Rescue Cat » : quatorze versions, sept raretés en deux
    /// illustrations — la page du 2026-10-21 et les prix du guide.
    #[test]
    fn deux_illustrations_relevees_ont_chacune_leur_page() {
        let ext = ext_de_test(5604, Region::Tcg);
        let ids = vec![
            769_676, 769_677, 769_757, 769_758, 769_838, 769_839, 769_919, 769_920, 770_007,
            770_008, 770_088, 770_089, 770_169, 770_170,
        ];
        let prix = tendances(&[
            (769_676, 22),
            (769_677, 17),
            (769_757, 20),
            (769_758, 23),
            (769_838, 59),
            (769_839, 48),
            (769_919, 109),
            (769_920, 79),
            (770_007, 349),
            (770_008, 206),
            (770_088, 68),
            (770_089, 58),
            (770_169, 86),
            (770_170, 93),
        ]);
        let mut classeur = Vec::new();
        for r in [
            "Collector's Rare",
            "Platinum Secret Rare",
            "Quarter Century Secret Rare",
            "Secret Rare",
            "Super Rare",
            "Ultimate Rare",
            "Ultra Rare",
        ] {
            classeur.push(r.to_owned());
            classeur.push(format!("{r} (illustration 2)"));
        }
        let connues = Connues {
            base: BTreeMap::new(),
            classeur,
        };
        let versions = Versions {
            ids,
            ajouts: Vec::new(),
        };
        let o = decider(
            &ext,
            &versions,
            &connues,
            "Platinum Secret Rare (illustration 2)",
            "Rescue Cat",
            &prix,
            String::new(),
        );
        assert_eq!(o.cible, Cible::Produit(769_920));
        assert_eq!(
            o.version.as_deref(),
            Some("V.8 · Platinum Secret Rare (illustration 2)")
        );
        let o = decider(
            &ext,
            &versions,
            &connues,
            "Quarter Century Secret Rare",
            "Rescue Cat",
            &prix,
            String::new(),
        );
        assert_eq!(o.cible, Cible::Produit(770_007));
        assert_eq!(
            o.version.as_deref(),
            Some("V.9 · Quarter Century Secret Rare")
        );
    }

    /// Les quatre versions de « Dark Magician Girl » : deux raretés, deux
    /// illustrations chacune.
    fn dark_magician_girl() -> (Versions, HashMap<i64, Prix>, Connues) {
        let versions = Versions {
            ids: vec![820_969, 820_970, 821_154, 821_155],
            ajouts: Vec::new(),
        };
        let prix = tendances(&[
            (820_969, 748),
            (820_970, 323),
            (821_154, 2378),
            (821_155, 1490),
        ]);
        let connues = Connues {
            base: BTreeMap::new(),
            classeur: cles(&[
                "Platinum Secret Rare",
                "Platinum Secret Rare (illustration 2)",
                "Quarter Century Secret Rare",
                "Quarter Century Secret Rare (illustration 2)",
            ]),
        };
        (versions, prix, connues)
    }

    /// RA04 : la page de « El Shaddoll Winda » range les illustrations — V.2
    /// est l'alternative Platinum —, mais le set a créé ses alternatives à
    /// part : on cherche, avec le numéro V.n, sans lien direct.
    #[test]
    fn ra04_le_rang_est_sur_le_produit_non() {
        let ext = ext_de_test(5989, Region::Tcg);
        let (versions, prix, connues) = dark_magician_girl();
        let o = decider(
            &ext,
            &versions,
            &connues,
            "Platinum Secret Rare (illustration 2)",
            "Dark Magician Girl",
            &prix,
            String::new(),
        );
        assert!(!o.est_directe());
        assert_eq!(o.raison, Raison::IllustrationsSaisiesAPart);
        assert_eq!(
            o.version.as_deref(),
            Some("V.2 · Platinum Secret Rare (illustration 2)")
        );
        assert_eq!(
            o.prix,
            PrixOffre::Fourchette {
                min: 323,
                max: 2378
            }
        );
    }

    /// Sans page du set (RA03), deux illustrations d'une même rareté ne sont
    /// pas rangées : on cherche, sans numéro V.n.
    #[test]
    fn deux_illustrations_sans_page_font_chercher() {
        let ext = ext_de_test(5848, Region::Tcg);
        let (versions, prix, connues) = dark_magician_girl();
        let o = decider(
            &ext,
            &versions,
            &connues,
            "Platinum Secret Rare (illustration 2)",
            "Dark Magician Girl",
            &prix,
            String::new(),
        );
        assert!(!o.est_directe());
        assert_eq!(o.raison, Raison::PlusieursVersionsDeLaRarete);
        assert_eq!(o.version, None);
    }

    /// RA05 « Raigeki » : les deux alternatives en tête (V.1 Ultra, V.2
    /// Starlight), créées cinq jours avant les sept autres versions — dans
    /// l'ordre des V.n, comme la page et les prix le montrent.
    #[test]
    fn ra05_les_alternatives_en_tete() {
        let ext = ext_de_test(6424, Region::Tcg);
        let versions = Versions {
            ids: vec![
                881_339, 881_340, 882_204, 882_206, 882_208, 882_209, 882_211, 882_212, 882_213,
            ],
            ajouts: Vec::new(),
        };
        let prix = tendances(&[
            (881_339, 63),
            (881_340, 310),
            (882_204, 20),
            (882_206, 29),
            (882_208, 57),
            (882_209, 136),
            (882_211, 375),
            (882_212, 218),
            (882_213, 119),
        ]);
        let connues = Connues {
            base: BTreeMap::new(),
            classeur: cles(&[
                "Collector's Rare",
                "Platinum Secret Rare",
                "Secret Rare",
                "Starlight Rare",
                "Starlight Rare (illustration 2)",
                "Super Rare",
                "Ultimate Rare",
                "Ultra Rare",
                "Ultra Rare (illustration 2)",
            ]),
        };
        let attendus = [
            ("Ultra Rare (illustration 2)", 881_339, 1),
            ("Starlight Rare (illustration 2)", 881_340, 2),
            ("Super Rare", 882_204, 3),
            ("Starlight Rare", 882_211, 7),
            ("Collector's Rare", 882_213, 9),
        ];
        for (cle, id, v) in attendus {
            let o = decider(
                &ext,
                &versions,
                &connues,
                cle,
                "Raigeki",
                &prix,
                String::new(),
            );
            assert_eq!(o.cible, Cible::Produit(id), "{cle}");
            assert_eq!(o.raison, Raison::Ordonnee, "{cle}");
            assert!(
                o.version
                    .as_deref()
                    .is_some_and(|t| t.starts_with(&format!("V.{v} · "))),
                "{cle} : {:?}",
                o.version
            );
        }
    }

    #[test]
    fn cles_de_version_par_les_fichiers() {
        let ligne = |rowid: i64, rarete: &str, fichier: &str, overframe: bool| LigneDeClasseur {
            rowid,
            rarete: rarete.to_owned(),
            fichier: fichier.to_owned(),
            overframe,
        };
        // Deux fichiers `-AA` pour deux lignes : indistinctes, elles gardent
        // leur rareté.
        let cdi = [
            ligne(
                1,
                "Quarter Century Secret Rare",
                "CDI-RA04-EN-QCScR-1E-AA.png",
                false,
            ),
            ligne(
                2,
                "Quarter Century Secret Rare",
                "CDI-RA04-EN-QCScR-1E-AA.png",
                false,
            ),
            ligne(3, "Super Rare", "CDI-RA04-EN-SR-1E.png", false),
        ];
        assert_eq!(
            cles_de_version("RA04-EN068", &cdi),
            cles(&[
                "Quarter Century Secret Rare",
                "Quarter Century Secret Rare",
                "Super Rare"
            ])
        );
        // Une image YGOPRODeck est l'illustration d'origine ; les chiffres
        // de LDK2 sont des rangs.
        let ldk2 = [
            ligne(1, "Common", "89631139.jpg", false),
            ligne(2, "Common", "BlueEyesWhiteDragon-LDK2-EN-C-1E-2.png", false),
            ligne(3, "Common", "BlueEyesWhiteDragon-LDK2-EN-C-1E-3.png", false),
        ];
        assert_eq!(
            cles_de_version("LDK2-ENK01", &ldk2),
            cles(&[
                "Common",
                "Common (illustration 2)",
                "Common (illustration 3)"
            ])
        );
        // La marque « illustration étendue » suffit, sans suffixe `-EA`.
        let locr = [
            ligne(1, "Ultra Rare", "X-LOCR-JP-UR.png", false),
            ligne(2, "Ultra Rare", "x.jpg", true),
            ligne(3, "Prismatic Secret Rare", "X-LOCR-JP-PScR.png", true),
        ];
        assert_eq!(
            cles_de_version("LOCR-JP001", &locr),
            cles(&[
                "Ultra Rare",
                "Ultra Rare (Overframe)",
                "Prismatic Secret Rare"
            ])
        );
        // Un suffixe qui ne dit rien de sûr (`J`) : on ne distingue pas.
        let poly = [
            ligne(1, "Common", "Polymerization-LDK2-EN-C-1E-J.png", false),
            ligne(2, "Common", "Polymerization-LDK2-EN-C-1E-K.png", false),
        ];
        assert_eq!(
            cles_de_version("LDK2-ENJ26", &poly),
            cles(&["Common", "Common"])
        );
    }

    #[test]
    fn nombre_de_versions_different() {
        let index = IndexCharge::new(index_de_test());
        let ext = index.extensions_du_set("set-a", Region::Ocg)[0];
        let codes = BTreeMap::from([(
            "AAA-JP001".to_owned(),
            BTreeSet::from(["Ultra Rare".to_owned(), "Secret Rare".to_owned()]),
        )]);
        let o = decider(
            ext,
            &ext.cartes["alpha"],
            &connues(codes.clone()),
            "Ultra Rare",
            "Alpha",
            &index.index.prix,
            String::new(),
        );
        assert_eq!(
            o.raison,
            Raison::NombreDeVersions {
                cardmarket: 1,
                base: 2
            }
        );
    }

    #[test]
    fn l_index_fait_l_aller_retour_sur_le_disque() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(dir.path());
        std::fs::create_dir_all(paths.bdd()).unwrap();
        let index = index_de_test();
        ecrire(&paths, &index).unwrap();
        let relu = lire(&paths).unwrap();
        assert_eq!(relu, index);
        let c = charge(&paths).unwrap();
        assert_eq!(c.index.extensions.len(), 2);
    }

    #[test]
    fn euros_a_la_francaise() {
        assert_eq!(euros(0), "0,00 €");
        assert_eq!(euros(5), "0,05 €");
        assert_eq!(euros(100_000_000), "1 000 000,00 €");
    }

    #[test]
    fn libelles_d_extension() {
        assert_eq!(
            libelle_extension(&[
                "Rarity Collection 5 Booster Box",
                "24x Rarity Collection 5 Booster",
                "Rarity Collection 5 Display"
            ])
            .as_deref(),
            Some("Rarity Collection 5")
        );
        assert_eq!(libelle_extension(&[]), None);
    }

    #[test]
    fn centimes_bornes() {
        assert_eq!(centimes(Some(5.494)), Some(549));
        assert_eq!(centimes(Some(-1.0)), None);
        assert_eq!(centimes(Some(f64::NAN)), None);
        assert_eq!(centimes(None), None);
    }

    /// Le nom normalisé, comme la base le range.
    fn tirer(b: &mut Base, sl: i64, nom: &str, code: &str, rarete: &str) {
        b.tirages
            .entry(sl)
            .or_default()
            .entry(regles::normaliser_nom(nom))
            .or_default()
            .entry(code.to_owned())
            .or_default()
            .insert(rarete.to_owned());
    }

    /// CORI (avril 2026) : 48 cartes en base, sous des noms anglais
    /// provisoires, 96 chez Cardmarket — les cartes ne rapprochent pas
    /// l'extension, le nom du set si. Pas l'extension coréenne aux mêmes
    /// cartes.
    #[test]
    fn un_set_ocg_recent_se_rapproche_par_son_nom() {
        let mut b = base();
        b.locales.insert(
            3,
            Locale {
                set_uuid: "set-b".into(),
                langue: "jp".into(),
                prefixe: "BBB-JP".into(),
                sortie: Some("2026-04-25".into()),
            },
        );
        b.noms_de_sets
            .insert("set-b".into(), regles::normaliser_nom("Chaos Origins"));
        for (nom, code) in [
            ("Eta", "BBB-JP001"),
            ("Theta", "BBB-JP002"),
            ("Iota", "BBB-JP003"),
            ("Kappa the Provisional Name", "BBB-JP004"),
        ] {
            tirer(&mut b, 3, nom, code, "Common");
        }
        let d = "2026-04-28 10:00:00";
        let produits = || {
            let mut produits = Vec::new();
            for (ext, depart) in [(30, 300), (40, 400)] {
                for (i, nom) in ["Eta", "Theta", "Iota"].iter().enumerate() {
                    produits.push(produit(depart + i as i64, nom, ext, d));
                }
                // Cardmarket a bien plus de cartes : Jaccard sous le seuil.
                for i in 0..10 {
                    produits.push(produit(depart + 10 + i, &format!("Lambda {i}"), ext, d));
                }
            }
            catalogue(produits)
        };
        let scelles = catalogue(vec![
            produit(990, "Chaos Origins (OCG) Booster Box", 30, d),
            produit(991, "Chaos Origins (Korean) Booster Box", 40, d),
        ]);
        let index = assembler(
            &produits(),
            &scelles,
            &GuidePrix::default(),
            &b,
            Empreinte::default(),
        );
        let cori = index.extensions.iter().find(|e| e.id == 30).unwrap();
        assert_eq!(cori.prefixe, "BBB-JP");
        assert_eq!(cori.region, Region::Ocg);
        assert_eq!(cori.libelle.as_deref(), Some("Chaos Origins (OCG)"));
        assert!(cori.jaccard < JACCARD_MINIMUM);
        assert!(index.extensions.iter().all(|e| e.id != 40));

        // Sans le nom du set, les cartes seules ne suffisent pas.
        b.noms_de_sets.clear();
        let index = assembler(
            &produits(),
            &scelles,
            &GuidePrix::default(),
            &b,
            Empreinte::default(),
        );
        assert!(index.extensions.iter().all(|e| e.id != 30));
    }

    /// Une base minimale : les tables que [`IndexCharge::offre`] interroge.
    fn cardinfo_de_test() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE set_locales (id INTEGER PRIMARY KEY, set_uuid TEXT, language TEXT);
             CREATE TABLE set_prints (set_locale_id INTEGER, card_uuid TEXT, set_code TEXT,
                                      rarity TEXT);
             CREATE TABLE card_texts (card_uuid TEXT, language TEXT, name TEXT);
             INSERT INTO set_locales VALUES (1, 'set-a', 'en'), (5, 'set-z', 'jp');
             INSERT INTO set_prints VALUES
                 (1, 'u-alpha', 'AAA-EN001', 'Ultra Rare'),
                 (1, 'u-alpha', 'AAA-EN001', 'Secret Rare'),
                 (1, 'u-omega', 'AAA-EN099', 'Common'),
                 (5, 'u-zed', 'ZZZ-JP001', 'Rare');
             INSERT INTO card_texts VALUES
                 ('u-alpha', 'en', 'Alpha'), ('u-omega', 'en', 'Omega'), ('u-zed', 'en', 'Zed');",
        )
        .unwrap();
        conn
    }

    #[test]
    fn la_carte_non_retrouvee_se_cherche_par_son_numero() {
        let index = IndexCharge::new(index_de_test());
        let conn = cardinfo_de_test();
        let offre = |set_code: &str, rarete: &str, nom: &str| {
            index
                .offre(
                    &conn,
                    &Demande {
                        set_code,
                        rarete,
                        nom,
                        card_uuid: None,
                        raretes: &[],
                    },
                )
                .unwrap()
        };
        // Retrouvée : la page du produit.
        assert_eq!(
            offre("AAA-EN001", "Secret Rare", "Alpha").cible,
            Cible::Produit(101)
        );
        // Un numéro que la base ignore, écrit en français : son numéro anglais.
        let o = offre("AAA-FR007", "Common", "Promo");
        assert_eq!(o.cible, Cible::Numero("AAA-EN007".into()));
        assert_eq!(o.raison, Raison::NumeroInconnu);
        assert!(o
            .url("fr")
            .contains("Search?searchMode=v2&idCategory=0&idExpansion=0&searchString=AAA-EN007&"));
        // Un set non rapproché.
        let o = offre("ZZZ-JP001", "Rare", "Zed");
        assert_eq!(o.cible, Cible::Numero("ZZZ-JP001".into()));
        assert_eq!(o.raison, Raison::ExtensionInconnue);
        // Une carte absente de l'extension : le numéro, l'extension nommée.
        let o = offre("AAA-EN099", "Common", "Omega");
        assert_eq!(o.cible, Cible::Numero("AAA-EN099".into()));
        assert_eq!(o.raison, Raison::CarteAbsente);
        assert_eq!(o.extension.as_deref(), Some("Set A"));
        // Un code qui ne s'écrit pas en numéro : le nom, faute de mieux.
        let o = offre("PROMO", "Common", "Promo");
        assert_eq!(
            o.cible,
            Cible::Recherche {
                nom: "Promo".into(),
                id_extension: None
            }
        );
        assert!(o.raison.carte_non_retrouvee());
    }

    #[test]
    fn la_valeur_ne_compte_que_les_prix_surs() {
        let index = IndexCharge::new(index_de_test());
        let cardinfo = cardinfo_de_test();
        let classeur = Connection::open_in_memory().unwrap();
        classeur
            .execute_batch(
                "CREATE TABLE cards (name TEXT, set_code TEXT, rarity TEXT, quantite INTEGER,
                                     sort_order INTEGER);
                 INSERT INTO cards VALUES
                     ('Alpha', 'AAA-EN001', 'Secret Rare', 2, 1),
                     ('Alpha', 'AAA-EN001', 'Ultra Rare', 0, 2),
                     ('Omega', 'AAA-EN099', 'Common', 3, 3),
                     ('Zed', 'ZZZ-JP001', 'Rare', 1, 4);",
            )
            .unwrap();
        let toutes = offres_du_classeur(&index, &cardinfo, &classeur, false).unwrap();
        assert_eq!(toutes.len(), 4);
        let possedees = offres_du_classeur(&index, &cardinfo, &classeur, true).unwrap();
        assert_eq!(possedees.len(), 3);
        // La ligne sans exemplaire nomme quand même les versions du numéro :
        // la Secret possédée garde sa page.
        assert_eq!(possedees[0].offre.cible, Cible::Produit(101));
        let mut v = Valeur::default();
        for l in &possedees {
            v.ajouter(l.quantite, l.offre.prix_unitaire());
        }
        // Alpha Secret : tendance 9,00 €, plus bas 4,50 €, pas de moyenne —
        // le médian, 6,75 €, deux fois. Omega et Zed, non retrouvées : pas
        // chiffrées.
        assert_eq!(
            v,
            Valeur {
                centimes: 1_350,
                chiffres: 2,
                non_chiffres: 4
            }
        );
    }
}
