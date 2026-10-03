// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Lignes de la base de référence, telles qu'elles circulent entre le parsing
//! et l'insertion.
//!
//! En Python, ce sont des tuples anonymes et des dictionnaires : `cards_rows`
//! est une liste de tuples de 13 éléments, `prints_rows_raw` une liste de
//! `dict`. Toute la chaîne — parsing, enrichissement, insertion — indexe ces
//! tuples par position (`row[4]` pour `frame_type`, `row[1]` pour le mot de
//! passe). C'est la source d'erreur la plus mécanique du pipeline, et elle
//! disparaît ici : chaque champ a un nom et un type.
//!
//! Ces structures sont **pures** : aucune ne connaît SQLite ni le réseau. Elles
//! sont produites par `ygo-sources` et consommées par `ygo-db` (règle R1).

/// Une ligne de la table `cards`.
///
/// Ordre des colonnes : `uuid`, `ygoprodeck_id`, `card_type`, `subcategory`,
/// `frame_type`, `atk`, `def`, `level`, `attribute`, `race`, `banlist_tcg`,
/// `banlist_ocg`, `name_fr_confirmed`.
///
/// À la sortie du parsing YGOJSON, les huit colonnes de statistiques sont
/// `None` ; elles sont remplies par l'enrichissement YGOPRODeck. Une carte est
/// dite « enrichie » quand `frame_type` n'est plus `None` — c'est exactement le
/// critère du compteur journalisé par la V1.0.4 (`sum(1 for r in cards_rows if
/// r[4] is not None)`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LigneCarte {
    /// UUID YGOJSON de la carte.
    pub uuid: String,
    /// « Password » Konami — le premier trouvé dans les images, sinon dans
    /// `passwords`.
    pub ygoprodeck_id: Option<i64>,
    /// `monster`, `spell`, `trap`…
    pub card_type: String,
    /// Sous-catégorie YGOJSON.
    pub subcategory: String,
    /// Type de cadre YGOPRODeck. `None` tant que la carte n'est pas enrichie.
    pub frame_type: Option<String>,
    /// Attaque.
    pub atk: Option<i64>,
    /// Défense (colonne SQL `def`).
    pub def: Option<i64>,
    /// Niveau / rang.
    pub level: Option<i64>,
    /// Attribut (LIGHT, DARK…).
    pub attribute: Option<String>,
    /// Type (Dragon, Spellcaster…).
    pub race: Option<String>,
    /// Statut de bannissement TCG.
    pub banlist_tcg: Option<String>,
    /// Statut de bannissement OCG.
    pub banlist_ocg: Option<String>,
    /// 1 si un nom français est confirmé par YGOJSON, 0 sinon.
    pub name_fr_confirmed: i64,
}

impl LigneCarte {
    /// La carte a-t-elle été enrichie par YGOPRODeck ?
    ///
    /// Critère repris tel quel de la V1.0.4 : `frame_type` renseigné.
    pub fn est_enrichie(&self) -> bool {
        self.frame_type.is_some()
    }
}

/// Une ligne de la table `card_texts`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LigneTexte {
    /// UUID de la carte.
    pub card_uuid: String,
    /// Code langue YGOJSON (`en`, `fr`, `ja`…).
    pub language: String,
    /// Nom de la carte dans cette langue.
    pub name: String,
    /// Texte d'effet, ou effet pendule à défaut.
    pub effect: String,
}

/// Une ligne de la table `card_images`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LigneImage {
    /// UUID de l'illustration.
    pub uuid: String,
    /// UUID de la carte.
    pub card_uuid: String,
    /// « Password » Konami de cette illustration.
    pub ygoprodeck_image_id: Option<i64>,
    /// URL de l'artwork seul.
    pub art_url: String,
    /// URL de la carte entière.
    pub card_url: String,
}

/// Une ligne de la table `sets`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LigneSet {
    /// UUID du set.
    pub uuid: String,
    /// Nom anglais.
    pub name_en: String,
    /// Nom français — **repli sur l'anglais** quand il manque, comme en Python.
    pub name_fr: String,
    /// Nom allemand.
    pub name_de: String,
    /// Nom italien.
    pub name_it: String,
    /// Nom espagnol.
    pub name_es: String,
    /// Nom japonais.
    pub name_ja: String,
    /// Nom coréen.
    pub name_ko: String,
}

/// Une ligne de la table `set_locales`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LigneLocale {
    /// UUID du set.
    pub set_uuid: String,
    /// Code langue de la déclinaison (`en`, `eu`, `jp`…).
    pub language: String,
    /// Préfixe des codes de set (`RA05-EN`).
    pub prefix: String,
    /// Date de sortie.
    pub release_date: String,
    /// URL de la cover du booster.
    pub booster_image_url: String,
}

/// Un tirage avant résolution de `set_locale_id`.
///
/// Correspond au `dict` de `prints_rows_raw` en Python. La colonne
/// `set_locale_id` de `set_prints` est un entier auto-incrémenté, inconnu tant
/// que `set_locales` n'a pas été insérée : le tirage garde donc `locale_key`,
/// et la résolution se fait au moment de l'insertion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TirageBrut {
    /// UUID du set.
    pub set_uuid: String,
    /// Code langue, à résoudre en `set_locale_id`.
    pub locale_key: String,
    /// UUID de la carte.
    pub card_uuid: String,
    /// UUID de l'illustration, résolu par l'expansion multi-artworks.
    pub card_image_uuid: Option<String>,
    /// Code de set complet (`RA05-EN134`).
    pub set_code: Option<String>,
    /// Rareté normalisée (libellé Scanflip).
    pub rarity: String,
    /// Édition (`1st`, `unlimited`…).
    pub edition: String,
    /// Quantité.
    pub qty: i64,
    /// URL d'image propre à ce tirage (Yugipedia via YGOJSON).
    pub print_image_url: Option<String>,
}

/// Résultat du parsing de `cards.json`.
#[derive(Debug, Clone, Default)]
pub struct CartesParsees {
    /// Lignes de `cards`.
    pub cartes: Vec<LigneCarte>,
    /// Lignes de `card_texts`.
    pub textes: Vec<LigneTexte>,
    /// Lignes de `card_images`.
    pub images: Vec<LigneImage>,
    /// UUID des cartes ayant un nom français confirmé.
    pub uuids_fr_confirmes: std::collections::BTreeSet<String>,
}

/// Résultat du parsing de `sets.json`.
#[derive(Debug, Clone, Default)]
pub struct SetsParses {
    /// Lignes de `sets`.
    pub sets: Vec<LigneSet>,
    /// Lignes de `set_locales`.
    pub locales: Vec<LigneLocale>,
    /// Tirages, avant résolution de `set_locale_id`.
    pub tirages: Vec<TirageBrut>,
}
