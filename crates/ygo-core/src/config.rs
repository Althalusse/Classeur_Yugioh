// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Préférences utilisateur — `bdd/app_config.json`.
//!
//! Portage de `module/app_config.py`, `module/config/preferences.py`,
//! `module/config_langue.py` et de la partie « source d'images » de
//! `module/config_image_source.py`.
//!
//! # Règle d'or, reprise du Python
//!
//! **Une configuration absente, corrompue ou hors bornes ne produit jamais une
//! erreur bloquante** : elle produit la valeur par défaut (§3.5 du cahier des
//! charges). C'est vérifiable — le `app_config.json` réel de la V1.0.3 ne
//! contient qu'une seule clé (`font_scale`), et l'application démarre.
//!
//! Seule l'**écriture** peut échouer et remonter une [`crate::CoreError`].
//!
//! # Clés reconnues
//!
//! | Clé | Type | Défaut | Garde-fou |
//! |---|---|---|---|
//! | `langue` | `"FR"` / `"EN"` | `FR` | valeur inconnue → défaut |
//! | `ui_langue` | `"FR"` / `"EN"` | `FR` | idem |
//! | `image_source` | `"YGOPRODECK"` / `"YUGIPEDIA"` | `YGOPRODECK` | idem |
//! | `font_scale` | nombre | `1.0` | borné à 0,5–3,0 |
//! | `grille_defaut` | `[cols, lignes]` | `[3, 3]` | chaque valeur bornée à 3–10 |
//! | `ordre_tri_criteres` | liste de 3 critères | `["numero","artwork","rarete"]` | doublons retirés, manquants complétés |
//! | `affichage_n_raretes_par_artwork` | entier | `0` (toutes) | borné à 0–20 |
//! | `affichage_une_rarete_par_artwork` | booléen | — | **rétrocompatibilité** : lu si la clé entière est absente (`true` → 1, `false` → 0) |
//! | `completer_artworks_yugipedia` | booléen | `true` | |
//! | `inclure_sets_ocg_jp` | booléen | — | **ignorée** : l'inclusion OCG-JP est forcée depuis la V1.0.4 |

use std::path::{Path, PathBuf};
use std::sync::RwLock;

use serde_json::{Map, Value};

use crate::error::{CoreError, Result};

// ─────────────────────────────────────────────────────────────────────────────
// Constantes de portage — valeurs reprises telles quelles du Python
// ─────────────────────────────────────────────────────────────────────────────

const CLE_LANGUE: &str = "langue";
const CLE_UI_LANGUE: &str = "ui_langue";
const CLE_IMAGE_SOURCE: &str = "image_source";
const CLE_FONT_SCALE: &str = "font_scale";
const CLE_GRILLE: &str = "grille_defaut";
const CLE_ORDRE_TRI: &str = "ordre_tri_criteres";
const CLE_N_RARETES: &str = "affichage_n_raretes_par_artwork";
/// Ancienne clé booléenne, lue en repli (`preferences._OLD_BOOL_KEY`).
const CLE_N_RARETES_LEGACY: &str = "affichage_une_rarete_par_artwork";
const CLE_COMPLETER_ARTWORKS: &str = "completer_artworks_yugipedia";

/// Plus petite grille acceptée (`preferences._GRILLE_MIN`).
pub const GRILLE_MIN: u8 = 3;
/// Plus grande grille acceptée (`preferences._GRILLE_MAX`).
pub const GRILLE_MAX: u8 = 10;
const GRILLE_DEFAUT: (u8, u8) = (3, 3);

/// `0` — toutes les raretés (`preferences._N_RARETES_MIN`).
pub const N_RARETES_MIN: u8 = 0;
/// Plafond du filtre « N raretés par artwork » (`preferences._N_RARETES_MAX`).
pub const N_RARETES_MAX: u8 = 20;
const N_RARETES_DEFAUT: u8 = 0;

// Les bornes de `font_scale` vivent côté Python dans `theme.py`, non dans
// `preferences.py` — d'où le repère provisoire (0,5–3,0) posé au premier lot,
// avec la note « à réaligner sur l'écran Options ». C'est fait : `theme.py`
// déclare `_FONT_SCALE_MIN = 0.85` et `_FONT_SCALE_MAX = 1.50`, et il **borne
// à la lecture** comme à l'écriture. Les valeurs d'attente auraient laissé
// passer un `2.0` que le Python ramène à `1.50`.
/// Plus petite échelle de police acceptée (`theme._FONT_SCALE_MIN`).
pub const FONT_SCALE_MIN: f64 = 0.85;
/// Plus grande échelle de police acceptée (`theme._FONT_SCALE_MAX`).
pub const FONT_SCALE_MAX: f64 = 1.50;
const FONT_SCALE_DEFAUT: f64 = 1.0;

/// Suffixes de code-langue OCG reconnus (`preferences._OCG_SUFFIXES`).
///
/// Sert à distinguer un classeur OCG (`LOCH-JP`) d'un classeur TCG, pour ne pas
/// lui appliquer les transformations propres au TCG.
pub const SUFFIXES_OCG: [&str; 7] = ["JP", "JA", "KR", "KO", "AE", "SC", "TC"];

// ─────────────────────────────────────────────────────────────────────────────
// Types de valeurs
// ─────────────────────────────────────────────────────────────────────────────

/// Langue de l'interface ou des noms de cartes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Langue {
    /// Français — valeur par défaut du Python.
    #[default]
    Fr,
    /// Anglais.
    En,
}

impl Langue {
    /// Code tel qu'il est stocké dans `app_config.json` (`"FR"` / `"EN"`).
    pub fn code(self) -> &'static str {
        match self {
            Self::Fr => "FR",
            Self::En => "EN",
        }
    }

    /// Analyse un code stocké. Toute valeur inconnue retombe sur le défaut.
    pub fn depuis_code(code: &str) -> Self {
        match code {
            "EN" => Self::En,
            _ => Self::Fr,
        }
    }

    /// Colonne SQL portant le nom de carte dans une base de classeur.
    ///
    /// Portage de `config_langue.get_name_column()` : `FR` → `name_fr`
    /// (avec repli applicatif sur `name` si vide), `EN` → `name`.
    pub fn colonne_nom(self) -> &'static str {
        match self {
            Self::Fr => "name_fr",
            Self::En => "name",
        }
    }
}

/// Source des images de cartes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SourceImage {
    /// `images.ygoprodeck.com` — JPEG HD, disponibilité maximale. Défaut.
    #[default]
    Ygoprodeck,
    /// `ms.yugipedia.com` — PNG par tirage, artwork exact par rareté.
    Yugipedia,
}

impl SourceImage {
    /// Code tel qu'il est stocké dans `app_config.json`.
    pub fn code(self) -> &'static str {
        match self {
            Self::Ygoprodeck => "YGOPRODECK",
            Self::Yugipedia => "YUGIPEDIA",
        }
    }

    /// Analyse un code stocké. Toute valeur inconnue retombe sur le défaut.
    pub fn depuis_code(code: &str) -> Self {
        match code {
            "YUGIPEDIA" => Self::Yugipedia,
            _ => Self::Ygoprodeck,
        }
    }
}

/// Critère de tri des cartes d'un classeur.
///
/// L'utilisateur en réordonne les trois par glisser-déposer dans les Options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CritereTri {
    /// Numéro extrait du `set_code`.
    Numero,
    /// Priorité de rareté (cf. [`crate::rarity`]).
    Rarete,
    /// Rang d'artwork — Art A avant Art B.
    Artwork,
}

impl CritereTri {
    /// Libellé stocké dans `app_config.json`.
    pub fn code(self) -> &'static str {
        match self {
            Self::Numero => "numero",
            Self::Rarete => "rarete",
            Self::Artwork => "artwork",
        }
    }

    /// Analyse un libellé stocké. `None` si inconnu.
    pub fn depuis_code(code: &str) -> Option<Self> {
        match code {
            "numero" => Some(Self::Numero),
            "rarete" => Some(Self::Rarete),
            "artwork" => Some(Self::Artwork),
            _ => None,
        }
    }
}

/// Ordre par défaut des critères (`preferences._TRI_DEFAULT`).
///
/// L'ordre importe : « artwork » est comparé **avant** « rarete », ce qui est
/// la raison d'être du regroupement des artworks externes sur un rang unique
/// dans `tri_carte._compute_art_ranks`.
pub const ORDRE_TRI_DEFAUT: [CritereTri; 3] =
    [CritereTri::Numero, CritereTri::Artwork, CritereTri::Rarete];

/// Ordre canonique servant à compléter une liste incomplète
/// (`preferences._TRI_CRITERES`).
const ORDRE_TRI_CANONIQUE: [CritereTri; 3] =
    [CritereTri::Numero, CritereTri::Rarete, CritereTri::Artwork];

// ─────────────────────────────────────────────────────────────────────────────
// Configuration
// ─────────────────────────────────────────────────────────────────────────────

/// Accès à `bdd/app_config.json`, avec cache mémoire.
///
/// Reprend le cache de `app_config.py` : le fichier est lu une fois, puis
/// maintenu en mémoire ; chaque écriture met le cache à jour **après** que
/// l'écriture disque a réussi.
#[derive(Debug)]
pub struct Config {
    chemin: PathBuf,
    cache: RwLock<Map<String, Value>>,
}

impl Config {
    /// Charge la configuration depuis le chemin donné.
    ///
    /// Ne peut pas échouer : fichier absent, illisible ou JSON invalide
    /// donnent une configuration vide, donc toutes les valeurs par défaut.
    pub fn charger(chemin: impl Into<PathBuf>) -> Self {
        let chemin = chemin.into();
        let cache = RwLock::new(Self::lire_disque(&chemin));
        Self { chemin, cache }
    }

    /// Configuration en mémoire, sans fichier associé — pour les tests.
    pub fn en_memoire() -> Self {
        Self {
            chemin: PathBuf::new(),
            cache: RwLock::new(Map::new()),
        }
    }

    fn lire_disque(chemin: &Path) -> Map<String, Value> {
        let Ok(texte) = std::fs::read_to_string(chemin) else {
            return Map::new();
        };
        match serde_json::from_str::<Value>(&texte) {
            Ok(Value::Object(map)) => map,
            _ => {
                tracing::warn!(
                    chemin = %chemin.display(),
                    "app_config.json illisible ou de forme inattendue — valeurs par défaut appliquées"
                );
                Map::new()
            }
        }
    }

    /// Force la relecture du fichier au prochain accès.
    ///
    /// Équivalent de `app_config.invalidate_cache()`.
    pub fn recharger(&self) {
        let frais = Self::lire_disque(&self.chemin);
        if let Ok(mut cache) = self.cache.write() {
            *cache = frais;
        }
    }

    /// Valeur brute associée à une clé.
    pub fn brut(&self, cle: &str) -> Option<Value> {
        self.cache.read().ok()?.get(cle).cloned()
    }

    /// Écrit `cle = valeur` en **préservant toutes les autres clés**.
    ///
    /// Le cache n'est mis à jour que si l'écriture disque a réussi ; en cas
    /// d'échec il est invalidé, comme en Python, pour éviter de servir un état
    /// divergent du fichier.
    pub fn definir(&self, cle: &str, valeur: Value) -> Result<()> {
        let mut donnees = match self.cache.read() {
            Ok(cache) => cache.clone(),
            Err(_) => Map::new(),
        };
        donnees.insert(cle.to_owned(), valeur);

        if let Some(parent) = self.chemin.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| CoreError::io(parent, e))?;
            }
        }
        let texte = serde_json::to_string_pretty(&Value::Object(donnees.clone()))
            .map_err(|e| CoreError::json(&self.chemin, e))?;
        std::fs::write(&self.chemin, texte).map_err(|e| {
            self.recharger();
            CoreError::io(&self.chemin, e)
        })?;

        if let Ok(mut cache) = self.cache.write() {
            *cache = donnees;
        }
        Ok(())
    }

    /// Retire une clé du fichier — le réglage **revient au défaut de
    /// l'application**.
    ///
    /// # Pourquoi effacer plutôt qu'écrire le défaut
    ///
    /// Écrire `[3, 3]` figerait la valeur d'aujourd'hui : si le défaut de
    /// l'application changeait, l'installation resterait à 3×3 sans que rien
    /// ne le dise. C'est la même règle qu'au niveau du classeur, où décocher
    /// « suivre les Options » efface la clé au lieu d'y recopier le défaut du
    /// moment. Un niveau qui ne décide pas doit être **absent**, pas égal.
    ///
    /// Effacer une clé absente n'est pas une erreur : le réglage suit déjà le
    /// défaut, et l'appel est sans effet.
    ///
    /// # Errors
    ///
    /// Si `app_config.json` n'a pas pu être écrit.
    pub fn effacer(&self, cle: &str) -> Result<()> {
        let mut donnees = match self.cache.read() {
            Ok(cache) => cache.clone(),
            Err(_) => Map::new(),
        };
        if donnees.remove(cle).is_none() {
            return Ok(());
        }
        if let Some(parent) = self.chemin.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| CoreError::io(parent, e))?;
            }
        }
        let texte = serde_json::to_string_pretty(&Value::Object(donnees.clone()))
            .map_err(|e| CoreError::json(&self.chemin, e))?;
        std::fs::write(&self.chemin, texte).map_err(|e| {
            self.recharger();
            CoreError::io(&self.chemin, e)
        })?;
        if let Ok(mut cache) = self.cache.write() {
            *cache = donnees;
        }
        Ok(())
    }

    /// La grille globale revient au défaut de l'application.
    ///
    /// # Errors
    ///
    /// Voir [`effacer`](Self::effacer).
    pub fn oublier_grille_defaut(&self) -> Result<(u8, u8)> {
        self.effacer(CLE_GRILLE)?;
        Ok(self.grille_defaut())
    }

    /// L'ordre de tri global revient au défaut de l'application.
    ///
    /// # Errors
    ///
    /// Voir [`effacer`](Self::effacer).
    pub fn oublier_ordre_tri(&self) -> Result<[CritereTri; 3]> {
        self.effacer(CLE_ORDRE_TRI)?;
        Ok(self.ordre_tri())
    }

    /// Ce réglage est-il décidé par l'utilisateur, ou laissé au défaut ?
    ///
    /// Sert aux Options à n'offrir « revenir au défaut » que quand il y a
    /// quelque chose à défaire.
    #[must_use]
    pub fn est_defini(&self, cle: &str) -> bool {
        self.brut(cle).is_some()
    }

    /// La clé de la grille globale, pour [`est_defini`](Self::est_defini).
    #[must_use]
    pub fn cle_grille() -> &'static str {
        CLE_GRILLE
    }

    /// La clé de l'ordre de tri global, pour [`est_defini`](Self::est_defini).
    #[must_use]
    pub fn cle_ordre_tri() -> &'static str {
        CLE_ORDRE_TRI
    }

    // ── Accès typés ─────────────────────────────────────────────────────────

    /// Langue des noms de cartes. Défaut : `FR`.
    pub fn langue(&self) -> Langue {
        self.brut(CLE_LANGUE)
            .as_ref()
            .and_then(Value::as_str)
            .map_or(Langue::default(), Langue::depuis_code)
    }

    /// Langue de l'interface. Défaut : `FR`.
    pub fn ui_langue(&self) -> Langue {
        self.brut(CLE_UI_LANGUE)
            .as_ref()
            .and_then(Value::as_str)
            .map_or(Langue::default(), Langue::depuis_code)
    }

    /// Source des images. Défaut : `YGOPRODECK`.
    pub fn source_image(&self) -> SourceImage {
        self.brut(CLE_IMAGE_SOURCE)
            .as_ref()
            .and_then(Value::as_str)
            .map_or(SourceImage::default(), SourceImage::depuis_code)
    }

    /// Facteur d'échelle de la police. Défaut : `1.0`, borné à 0,85–1,50.
    pub fn font_scale(&self) -> f64 {
        self.brut(CLE_FONT_SCALE)
            .as_ref()
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite())
            .map_or(FONT_SCALE_DEFAUT, |v| {
                v.clamp(FONT_SCALE_MIN, FONT_SCALE_MAX)
            })
    }

    /// Grille par défaut `(colonnes, lignes)`. Défaut `(3, 3)`, bornée à 3–10.
    ///
    /// Accepte les deux formes rencontrées en Python : une liste `[cols, rows]`
    /// et un objet `{"cols": …, "rows": …}`.
    pub fn grille_defaut(&self) -> (u8, u8) {
        match self.brut(CLE_GRILLE) {
            Some(Value::Array(v)) if v.len() == 2 => {
                (borne_grille(v.first()), borne_grille(v.get(1)))
            }
            Some(Value::Object(o)) => (borne_grille(o.get("cols")), borne_grille(o.get("rows"))),
            _ => GRILLE_DEFAUT,
        }
    }

    /// Ordre des critères de tri.
    ///
    /// Portage de `preferences._clean_criteres` : les valeurs inconnues et les
    /// doublons sont retirés, puis les critères manquants sont ajoutés dans
    /// leur ordre canonique. Le résultat contient donc toujours exactement les
    /// trois critères.
    pub fn ordre_tri(&self) -> [CritereTri; 3] {
        let Some(Value::Array(items)) = self.brut(CLE_ORDRE_TRI) else {
            return ORDRE_TRI_DEFAUT;
        };
        let codes: Vec<&str> = items.iter().filter_map(Value::as_str).collect();
        normaliser_ordre_tri(&codes)
    }

    /// Nombre de raretés affichées par (numéro, artwork). `0` = toutes.
    ///
    /// Portage de `preferences.get_n_raretes_par_artwork`, **rétrocompatibilité
    /// comprise** : si la clé entière est absente, l'ancienne clé booléenne
    /// `affichage_une_rarete_par_artwork` est lue (`true` → 1, `false` → 0).
    /// Attention à l'ordre du test : en JSON comme en Python, un booléen ne
    /// doit pas être confondu avec un entier.
    pub fn n_raretes_par_artwork(&self) -> u8 {
        if let Some(v) = self.brut(CLE_N_RARETES) {
            if !v.is_boolean() {
                if let Some(n) = v.as_i64() {
                    return n.clamp(i64::from(N_RARETES_MIN), i64::from(N_RARETES_MAX)) as u8;
                }
            }
        }
        match self.brut(CLE_N_RARETES_LEGACY) {
            Some(Value::Bool(b)) => u8::from(b),
            Some(Value::Number(n)) => u8::from(n.as_i64().unwrap_or(0) != 0),
            _ => N_RARETES_DEFAUT,
        }
    }

    /// Complétion des artworks via Yugipedia. Défaut : **activée** (§3.5).
    pub fn completer_artworks_yugipedia(&self) -> bool {
        self.brut(CLE_COMPLETER_ARTWORKS)
            .as_ref()
            .and_then(Value::as_bool)
            .unwrap_or(true)
    }

    // ── Écritures typées ────────────────────────────────────────────────
    //
    // Chacune rend la valeur **effective**, c'est-à-dire ce qui a réellement
    // été écrit après bornage ou nettoyage. C'est le contrat du Python, et il
    // n'est pas décoratif : l'écran des options resynchronise ses champs sur
    // cette valeur, sinon l'utilisateur voit `99` dans une case où `10` a été
    // enregistré.

    /// Écrit la langue des noms de cartes.
    ///
    /// # Errors
    ///
    /// Si `app_config.json` n'a pas pu être écrit.
    pub fn definir_langue(&self, langue: Langue) -> Result<Langue> {
        self.definir(CLE_LANGUE, Value::from(langue.code()))?;
        Ok(langue)
    }

    /// Écrit la langue de l'interface.
    ///
    /// # Errors
    ///
    /// Si `app_config.json` n'a pas pu être écrit.
    pub fn definir_ui_langue(&self, langue: Langue) -> Result<Langue> {
        self.definir(CLE_UI_LANGUE, Value::from(langue.code()))?;
        Ok(langue)
    }

    /// Écrit la source d'images.
    ///
    /// # Errors
    ///
    /// Si `app_config.json` n'a pas pu être écrit.
    pub fn definir_source_image(&self, source: SourceImage) -> Result<SourceImage> {
        self.definir(CLE_IMAGE_SOURCE, Value::from(source.code()))?;
        Ok(source)
    }

    /// Écrit l'échelle de police, **bornée**.
    ///
    /// # Errors
    ///
    /// Si `app_config.json` n'a pas pu être écrit.
    pub fn definir_font_scale(&self, echelle: f64) -> Result<f64> {
        let effective = if echelle.is_finite() {
            echelle.clamp(FONT_SCALE_MIN, FONT_SCALE_MAX)
        } else {
            FONT_SCALE_DEFAUT
        };
        self.definir(CLE_FONT_SCALE, Value::from(effective))?;
        Ok(effective)
    }

    /// Écrit la grille par défaut, **bornée** à 3–10.
    ///
    /// Écrite sous forme de liste `[colonnes, lignes]` — la forme que le
    /// Python produit. L'objet `{"cols": …, "rows": …}` est encore *lu*, mais
    /// n'est plus écrit : mieux vaut une seule forme en sortie.
    ///
    /// # Errors
    ///
    /// Si `app_config.json` n'a pas pu être écrit.
    pub fn definir_grille_defaut(&self, colonnes: u8, lignes: u8) -> Result<(u8, u8)> {
        let borne = |v: u8| v.clamp(GRILLE_MIN, GRILLE_MAX);
        let (c, l) = (borne(colonnes), borne(lignes));
        self.definir(
            CLE_GRILLE,
            Value::from(vec![Value::from(c), Value::from(l)]),
        )?;
        Ok((c, l))
    }

    /// Écrit l'ordre des critères de tri, **nettoyé**.
    ///
    /// Le nettoyage est celui de la lecture : doublons retirés, critères
    /// manquants complétés dans l'ordre canonique. Une liste partielle venue
    /// d'un glisser-déposer interrompu ne peut donc pas s'installer en base.
    ///
    /// # Errors
    ///
    /// Si `app_config.json` n'a pas pu être écrit.
    pub fn definir_ordre_tri(&self, criteres: &[CritereTri]) -> Result<[CritereTri; 3]> {
        // La même règle qu'à la lecture, et la même qu'au niveau classeur :
        // une seule fonction, trois appelants. La recopier ici — ce qu'elle
        // faisait — c'était s'exposer à ce qu'une écriture et une lecture ne
        // s'accordent plus au premier correctif.
        let codes: Vec<&str> = criteres.iter().map(|c| c.code()).collect();
        let ordre = normaliser_ordre_tri(&codes);
        let liste: Vec<Value> = ordre.iter().map(|c| Value::from(c.code())).collect();
        self.definir(CLE_ORDRE_TRI, Value::from(liste))?;
        Ok(ordre)
    }

    /// Écrit le nombre de raretés par artwork, **borné** à 0–20.
    ///
    /// L'ancienne clé booléenne n'est pas touchée : la lecture ne la consulte
    /// que si la clé entière est absente, et elle ne l'est plus après cet
    /// appel. L'effacer serait une modification que le Python ne fait pas.
    ///
    /// # Errors
    ///
    /// Si `app_config.json` n'a pas pu être écrit.
    pub fn definir_n_raretes_par_artwork(&self, n: u8) -> Result<u8> {
        let effectif = n.clamp(N_RARETES_MIN, N_RARETES_MAX);
        self.definir(CLE_N_RARETES, Value::from(effectif))?;
        Ok(effectif)
    }

    /// Écrit la complétion des artworks via Yugipedia.
    ///
    /// # Errors
    ///
    /// Si `app_config.json` n'a pas pu être écrit.
    pub fn definir_completer_artworks_yugipedia(&self, actif: bool) -> Result<bool> {
        self.definir(CLE_COMPLETER_ARTWORKS, Value::from(actif))?;
        Ok(actif)
    }

    /// Codes-langue actifs pour filtrer `set_locales.language`.
    ///
    /// Portage de `preferences.get_langues_locales_actives()`. L'inclusion des
    /// sets OCG japonais est **forcée** depuis la V1.0.4 : la préférence
    /// `inclure_sets_ocg_jp` existe encore en base mais n'a plus d'effet.
    pub fn langues_locales_actives(&self) -> &'static [&'static str] {
        &["en", "eu", "jp"]
    }
}

/// Ramène une suite de codes à un ordre de tri complet et sans doublon.
///
/// Portage de `preferences._clean_criteres` : les valeurs inconnues et les
/// doublons sont retirés, puis les critères manquants sont ajoutés dans leur
/// ordre canonique. Le résultat contient donc **toujours** les trois critères.
///
/// # Pourquoi cette fonction est publique
///
/// L'ordre de tri se lit à deux endroits : les Options, pour toute
/// l'installation, et la table `meta` d'un classeur, pour lui seul. Écrire la
/// règle deux fois, c'est la voir diverger au premier correctif — le défaut
/// exact qui a fait revenir une carte sur la ligne d'un autre artwork
/// (cf. `export::rangs`). Une seule règle, deux lecteurs.
///
/// ```
/// use ygo_core::config::{normaliser_ordre_tri, CritereTri};
/// // Complété dans l'ordre canonique.
/// assert_eq!(
///     normaliser_ordre_tri(&["rarete"]),
///     [CritereTri::Rarete, CritereTri::Numero, CritereTri::Artwork]
/// );
/// // Doublons et inconnus écartés.
/// assert_eq!(
///     normaliser_ordre_tri(&["artwork", "artwork", "zzz", "numero"]),
///     [CritereTri::Artwork, CritereTri::Numero, CritereTri::Rarete]
/// );
/// // Rien d'exploitable : l'ordre **canonique** — qui n'est pas celui par
/// // défaut des Options (`numero, artwork, rarete`). Le premier est l'ordre
/// // dans lequel on complète, le second ce qu'un fichier vide vaut.
/// assert_eq!(
///     normaliser_ordre_tri(&[]),
///     [CritereTri::Numero, CritereTri::Rarete, CritereTri::Artwork]
/// );
/// ```
#[must_use]
pub fn normaliser_ordre_tri(codes: &[&str]) -> [CritereTri; 3] {
    let mut vus: Vec<CritereTri> = Vec::with_capacity(3);
    for code in codes {
        if let Some(c) = CritereTri::depuis_code(code) {
            if !vus.contains(&c) {
                vus.push(c);
            }
        }
    }
    for c in ORDRE_TRI_CANONIQUE {
        if !vus.contains(&c) {
            vus.push(c);
        }
    }
    [
        vus.first().copied().unwrap_or(CritereTri::Numero),
        vus.get(1).copied().unwrap_or(CritereTri::Rarete),
        vus.get(2).copied().unwrap_or(CritereTri::Artwork),
    ]
}

/// Borne une valeur de grille à 3–10, avec repli sur 3 si elle est inutilisable.
fn borne_grille(v: Option<&Value>) -> u8 {
    v.and_then(Value::as_i64).map_or(GRILLE_MIN, |n| {
        n.clamp(i64::from(GRILLE_MIN), i64::from(GRILLE_MAX)) as u8
    })
}

/// Indique si un nom de classeur ou un `set_code` porte un suffixe OCG.
///
/// Portage de `preferences.a_suffixe_ocg`.
///
/// ```
/// use ygo_core::config::a_suffixe_ocg;
/// assert!(a_suffixe_ocg("LOCH-JP"));
/// assert!(a_suffixe_ocg("LOCH-JP001"));
/// assert!(!a_suffixe_ocg("CROS"));
/// assert!(!a_suffixe_ocg("CROS-EN001"));
/// assert!(!a_suffixe_ocg(""));
/// ```
pub fn a_suffixe_ocg(code_ou_classeur: &str) -> bool {
    let s = code_ou_classeur.trim().to_ascii_uppercase();
    let Some((_, suffixe_brut)) = s.rsplit_once('-') else {
        return false;
    };
    let alpha: String = suffixe_brut
        .chars()
        .take_while(char::is_ascii_alphabetic)
        .collect();
    SUFFIXES_OCG.contains(&alpha.as_str())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;
    use serde_json::json;

    fn config_avec(valeur: Value) -> Config {
        let cfg = Config::en_memoire();
        if let Value::Object(map) = valeur {
            let mut cache = cfg.cache.write().unwrap();
            *cache = map;
        }
        cfg
    }

    // ── Règle d'or : jamais d'erreur bloquante ──────────────────────────────

    /// La cascade complète : défaut de l'application, règle globale, et
    /// retour au défaut.
    ///
    /// Le troisième niveau — la règle par classeur — vit dans la table `meta`
    /// et se teste dans `ygo_app::classeur`. Ce qui se vérifie ici, c'est que
    /// les deux premiers s'emboîtent **et se démontent**.
    #[test]
    fn la_regle_globale_se_pose_et_se_retire() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = Config::charger(tmp.path().join("app_config.json"));

        // Niveau 1 : rien n'est écrit, l'application décide.
        assert_eq!(cfg.grille_defaut(), (3, 3));
        assert_eq!(cfg.ordre_tri(), ORDRE_TRI_DEFAUT);
        assert!(!cfg.est_defini(Config::cle_grille()));
        assert!(!cfg.est_defini(Config::cle_ordre_tri()));

        // Niveau 2 : l'utilisateur décide autrement.
        assert_eq!(cfg.definir_grille_defaut(4, 5).unwrap(), (4, 5));
        let ordre = cfg
            .definir_ordre_tri(&[CritereTri::Rarete, CritereTri::Numero])
            .unwrap();
        assert_eq!(
            ordre,
            [CritereTri::Rarete, CritereTri::Numero, CritereTri::Artwork],
            "complété dans l'ordre canonique"
        );
        assert_eq!(cfg.grille_defaut(), (4, 5));
        assert!(cfg.est_defini(Config::cle_grille()));
        assert!(cfg.est_defini(Config::cle_ordre_tri()));

        // Retour au niveau 1 : la clé part, elle n'est pas remplie du défaut.
        assert_eq!(cfg.oublier_grille_defaut().unwrap(), (3, 3));
        assert_eq!(cfg.oublier_ordre_tri().unwrap(), ORDRE_TRI_DEFAUT);
        assert!(!cfg.est_defini(Config::cle_grille()));
        assert!(!cfg.est_defini(Config::cle_ordre_tri()));

        // Et c'est bien une absence sur le disque, pas une valeur écrite.
        let texte = std::fs::read_to_string(tmp.path().join("app_config.json")).unwrap();
        assert!(!texte.contains("grille_defaut"), "{texte}");
        assert!(!texte.contains("ordre_tri_criteres"), "{texte}");

        // Oublier ce qui n'est pas défini ne se plaint pas.
        assert!(cfg.oublier_grille_defaut().is_ok());
    }

    /// L'écriture et la lecture de l'ordre suivent **la même** règle.
    ///
    /// `definir_ordre_tri` portait sa propre copie du nettoyage. Deux
    /// jumelles finissent par diverger — c'est ce défaut qui, sur les rangs
    /// d'artwork, faisait revenir une carte sur la ligne d'une autre.
    #[test]
    fn ecrire_et_lire_l_ordre_donnent_le_meme_resultat() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = Config::charger(tmp.path().join("app_config.json"));
        for entree in [
            vec![CritereTri::Artwork],
            vec![CritereTri::Rarete, CritereTri::Rarete],
            vec![CritereTri::Artwork, CritereTri::Numero, CritereTri::Rarete],
            vec![],
        ] {
            let ecrit = cfg.definir_ordre_tri(&entree).unwrap();
            assert_eq!(ecrit, cfg.ordre_tri(), "entrée {entree:?}");
            let codes: Vec<&str> = entree.iter().map(|c| c.code()).collect();
            assert_eq!(ecrit, normaliser_ordre_tri(&codes));
        }
    }

    #[test]
    fn fichier_absent_donne_tous_les_defauts() {
        let cfg = Config::charger("/chemin/qui/n/existe/pas/app_config.json");

        assert_eq!(cfg.langue(), Langue::Fr);
        assert_eq!(cfg.ui_langue(), Langue::Fr);
        assert_eq!(cfg.source_image(), SourceImage::Ygoprodeck);
        assert!((cfg.font_scale() - 1.0).abs() < f64::EPSILON);
        assert_eq!(cfg.grille_defaut(), (3, 3));
        assert_eq!(cfg.ordre_tri(), ORDRE_TRI_DEFAUT);
        assert_eq!(cfg.n_raretes_par_artwork(), 0);
        assert!(cfg.completer_artworks_yugipedia());
    }

    #[test]
    fn fichier_corrompu_donne_tous_les_defauts() {
        let tmp = tempfile::tempdir().unwrap();
        let chemin = tmp.path().join("app_config.json");
        std::fs::write(&chemin, b"{ ceci n'est pas du JSON").unwrap();

        let cfg = Config::charger(&chemin);
        assert_eq!(cfg.grille_defaut(), (3, 3));
        assert_eq!(cfg.ordre_tri(), ORDRE_TRI_DEFAUT);
    }

    /// Cas réel : le `app_config.json` de la V1.0.3 ne contient qu'une clé.
    #[test]
    fn fichier_partiel_reel_v103() {
        let tmp = tempfile::tempdir().unwrap();
        let chemin = tmp.path().join("app_config.json");
        std::fs::write(
            &chemin,
            br#"{
  "font_scale": 1.15
}"#,
        )
        .unwrap();

        let cfg = Config::charger(&chemin);
        assert!((cfg.font_scale() - 1.15).abs() < 1e-9);
        // Tout le reste doit tomber sur son défaut, sans erreur.
        assert_eq!(cfg.langue(), Langue::Fr);
        assert_eq!(cfg.source_image(), SourceImage::Ygoprodeck);
        assert_eq!(cfg.grille_defaut(), (3, 3));
        assert_eq!(cfg.ordre_tri(), ORDRE_TRI_DEFAUT);
    }

    // ── Garde-fous ──────────────────────────────────────────────────────────

    #[test]
    fn grille_est_bornee_a_3_10() {
        assert_eq!(
            config_avec(json!({ "grille_defaut": [1, 99] })).grille_defaut(),
            (3, 10)
        );
        assert_eq!(
            config_avec(json!({ "grille_defaut": [4, 7] })).grille_defaut(),
            (4, 7)
        );
        assert_eq!(
            config_avec(json!({ "grille_defaut": [10, 10] })).grille_defaut(),
            (10, 10)
        );
        // Forme objet, rencontrée dans d'anciennes configurations.
        assert_eq!(
            config_avec(json!({ "grille_defaut": { "cols": 5, "rows": 2 } })).grille_defaut(),
            (5, 3)
        );
        // Formes inutilisables → défaut.
        assert_eq!(
            config_avec(json!({ "grille_defaut": "5x5" })).grille_defaut(),
            (3, 3)
        );
        assert_eq!(
            config_avec(json!({ "grille_defaut": [3] })).grille_defaut(),
            (3, 3)
        );
    }

    /// Les bornes sont celles de `theme.py` — 0,85 à 1,50 — et non les valeurs
    /// d'attente du premier lot. Le Python borne **à la lecture** aussi : une
    /// valeur de `99` déjà en base ne doit pas s'appliquer.
    #[test]
    fn font_scale_hors_bornes_retombe_dans_les_bornes() {
        let approche = |a: f64, b: f64| (a - b).abs() < 1e-9;
        assert!(approche(
            config_avec(json!({ "font_scale": 99.0 })).font_scale(),
            FONT_SCALE_MAX
        ));
        assert!(approche(
            config_avec(json!({ "font_scale": 0.0 })).font_scale(),
            FONT_SCALE_MIN
        ));
        assert!(approche(
            config_avec(json!({ "font_scale": "grand" })).font_scale(),
            1.0
        ));
        assert!(approche(FONT_SCALE_MIN, 0.85));
        assert!(approche(FONT_SCALE_MAX, 1.50));
        // La valeur réelle de l'utilisateur, qui tient dans les bornes.
        assert!(approche(
            config_avec(json!({ "font_scale": 1.15 })).font_scale(),
            1.15
        ));
    }

    #[test]
    fn ordre_tri_complete_et_dedoublonne() {
        // Liste incomplète : les critères manquants sont ajoutés dans l'ordre canonique.
        assert_eq!(
            config_avec(json!({ "ordre_tri_criteres": ["rarete"] })).ordre_tri(),
            [CritereTri::Rarete, CritereTri::Numero, CritereTri::Artwork]
        );
        // Doublons et valeurs inconnues : retirés.
        assert_eq!(
            config_avec(json!({ "ordre_tri_criteres": ["artwork", "artwork", "zzz", "numero"] }))
                .ordre_tri(),
            [CritereTri::Artwork, CritereTri::Numero, CritereTri::Rarete]
        );
        // Liste complète : respectée telle quelle.
        assert_eq!(
            config_avec(json!({ "ordre_tri_criteres": ["rarete", "artwork", "numero"] }))
                .ordre_tri(),
            [CritereTri::Rarete, CritereTri::Artwork, CritereTri::Numero]
        );
        // Type inattendu → défaut.
        assert_eq!(
            config_avec(json!({ "ordre_tri_criteres": "numero" })).ordre_tri(),
            ORDRE_TRI_DEFAUT
        );
    }

    #[test]
    fn n_raretes_est_borne_et_retrocompatible() {
        assert_eq!(
            config_avec(json!({ "affichage_n_raretes_par_artwork": 3 })).n_raretes_par_artwork(),
            3
        );
        assert_eq!(
            config_avec(json!({ "affichage_n_raretes_par_artwork": 99 })).n_raretes_par_artwork(),
            20
        );
        assert_eq!(
            config_avec(json!({ "affichage_n_raretes_par_artwork": -5 })).n_raretes_par_artwork(),
            0
        );

        // Ancienne clé booléenne, lue seulement si la nouvelle est absente.
        assert_eq!(
            config_avec(json!({ "affichage_une_rarete_par_artwork": true }))
                .n_raretes_par_artwork(),
            1
        );
        assert_eq!(
            config_avec(json!({ "affichage_une_rarete_par_artwork": false }))
                .n_raretes_par_artwork(),
            0
        );

        // La nouvelle clé a la priorité sur l'ancienne.
        assert_eq!(
            config_avec(json!({
                "affichage_n_raretes_par_artwork": 5,
                "affichage_une_rarete_par_artwork": true
            }))
            .n_raretes_par_artwork(),
            5
        );
    }

    #[test]
    fn langue_et_source_image_inconnues_retombent_sur_le_defaut() {
        assert_eq!(config_avec(json!({ "langue": "EN" })).langue(), Langue::En);
        assert_eq!(config_avec(json!({ "langue": "DE" })).langue(), Langue::Fr);
        assert_eq!(
            config_avec(json!({ "image_source": "YUGIPEDIA" })).source_image(),
            SourceImage::Yugipedia
        );
        assert_eq!(
            config_avec(json!({ "image_source": "FLICKR" })).source_image(),
            SourceImage::Ygoprodeck
        );
        assert_eq!(Langue::Fr.colonne_nom(), "name_fr");
        assert_eq!(Langue::En.colonne_nom(), "name");
    }

    // ── Écriture ────────────────────────────────────────────────────────────

    #[test]
    fn definir_preserve_les_autres_cles() {
        let tmp = tempfile::tempdir().unwrap();
        let chemin = tmp.path().join("bdd").join("app_config.json");

        let cfg = Config::charger(&chemin);
        cfg.definir(CLE_FONT_SCALE, json!(1.15)).unwrap();
        cfg.definir(CLE_LANGUE, json!("EN")).unwrap();
        cfg.definir(CLE_GRILLE, json!([4, 4])).unwrap();

        // Relecture à froid : les trois clés sont là.
        let relu = Config::charger(&chemin);
        assert!((relu.font_scale() - 1.15).abs() < 1e-9);
        assert_eq!(relu.langue(), Langue::En);
        assert_eq!(relu.grille_defaut(), (4, 4));
    }

    #[test]
    fn recharger_voit_une_modification_externe() {
        let tmp = tempfile::tempdir().unwrap();
        let chemin = tmp.path().join("app_config.json");
        std::fs::write(&chemin, br#"{"langue": "FR"}"#).unwrap();

        let cfg = Config::charger(&chemin);
        assert_eq!(cfg.langue(), Langue::Fr);

        std::fs::write(&chemin, br#"{"langue": "EN"}"#).unwrap();
        assert_eq!(
            cfg.langue(),
            Langue::Fr,
            "le cache doit masquer la modification"
        );

        cfg.recharger();
        assert_eq!(cfg.langue(), Langue::En);
    }

    // ── Suffixes OCG ────────────────────────────────────────────────────────

    #[test]
    fn suffixes_ocg() {
        assert!(a_suffixe_ocg("LOCH-JP"));
        assert!(a_suffixe_ocg("LOCR-JP001"));
        assert!(a_suffixe_ocg("xxx-kr"));
        assert!(!a_suffixe_ocg("CROS"));
        assert!(!a_suffixe_ocg("CROS-EN001"));
        assert!(!a_suffixe_ocg("RA05-FR134"));
        assert!(!a_suffixe_ocg(""));
    }

    // ── Écritures typées ────────────────────────────────────────────────────

    /// Une config posée sur un vrai fichier — les écritures y passent.
    fn config_sur_disque() -> (tempfile::TempDir, Config) {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = Config::charger(tmp.path().join("bdd").join("app_config.json"));
        (tmp, cfg)
    }

    /// Chaque écriture doit se relire à l'identique par son propre lecteur.
    /// C'est la seule propriété qui compte vraiment : un couple
    /// écriture/lecture qui diverge rend l'écran des options menteur.
    #[test]
    fn chaque_reglage_se_relit_comme_il_a_ete_ecrit() {
        let (_tmp, cfg) = config_sur_disque();

        cfg.definir_langue(Langue::En).unwrap();
        assert_eq!(cfg.langue(), Langue::En);

        cfg.definir_ui_langue(Langue::En).unwrap();
        assert_eq!(cfg.ui_langue(), Langue::En);

        cfg.definir_source_image(SourceImage::Yugipedia).unwrap();
        assert_eq!(cfg.source_image(), SourceImage::Yugipedia);

        cfg.definir_grille_defaut(4, 3).unwrap();
        assert_eq!(cfg.grille_defaut(), (4, 3));

        cfg.definir_n_raretes_par_artwork(2).unwrap();
        assert_eq!(cfg.n_raretes_par_artwork(), 2);

        cfg.definir_completer_artworks_yugipedia(false).unwrap();
        assert!(!cfg.completer_artworks_yugipedia());

        let ecrite = cfg.definir_font_scale(1.15).unwrap();
        assert!((cfg.font_scale() - ecrite).abs() < 1e-9);

        cfg.definir_ordre_tri(&[CritereTri::Rarete, CritereTri::Numero, CritereTri::Artwork])
            .unwrap();
        assert_eq!(
            cfg.ordre_tri(),
            [CritereTri::Rarete, CritereTri::Numero, CritereTri::Artwork]
        );
    }

    /// L'écriture rend la valeur **effective**, celle qui a réellement été
    /// enregistrée. Sans cela, l'écran des options laisserait `99` affiché
    /// dans une case où `10` a été écrit.
    #[test]
    fn l_ecriture_rend_la_valeur_effective_et_non_la_valeur_demandee() {
        let (_tmp, cfg) = config_sur_disque();

        assert_eq!(
            cfg.definir_grille_defaut(99, 1).unwrap(),
            (GRILLE_MAX, GRILLE_MIN)
        );
        assert_eq!(cfg.grille_defaut(), (GRILLE_MAX, GRILLE_MIN));

        assert_eq!(
            cfg.definir_n_raretes_par_artwork(99).unwrap(),
            N_RARETES_MAX
        );
        assert_eq!(cfg.n_raretes_par_artwork(), N_RARETES_MAX);

        let echelle = cfg.definir_font_scale(99.0).unwrap();
        assert!((echelle - FONT_SCALE_MAX).abs() < 1e-9);
        assert!((cfg.font_scale() - FONT_SCALE_MAX).abs() < 1e-9);

        let echelle = cfg.definir_font_scale(f64::NAN).unwrap();
        assert!(
            (echelle - 1.0).abs() < 1e-9,
            "un NaN ne s'installe pas en base"
        );
    }

    /// Un ordre partiel — glisser-déposer interrompu, liste tronquée — est
    /// complété avant écriture. Une liste incomplète ne peut pas s'installer.
    #[test]
    fn un_ordre_de_tri_partiel_est_complete_avant_d_etre_ecrit() {
        let (_tmp, cfg) = config_sur_disque();
        assert_eq!(
            cfg.definir_ordre_tri(&[CritereTri::Artwork]).unwrap(),
            [CritereTri::Artwork, CritereTri::Numero, CritereTri::Rarete]
        );
        assert_eq!(cfg.ordre_tri().len(), 3);

        // Doublons retirés, et une liste vide donne l'ordre canonique.
        assert_eq!(
            cfg.definir_ordre_tri(&[CritereTri::Rarete, CritereTri::Rarete])
                .unwrap(),
            [CritereTri::Rarete, CritereTri::Numero, CritereTri::Artwork]
        );
        assert_eq!(
            cfg.definir_ordre_tri(&[]).unwrap(),
            [CritereTri::Numero, CritereTri::Rarete, CritereTri::Artwork]
        );
    }

    /// Écrire un réglage ne doit pas en effacer un autre — ni les clés que
    /// cette application ne connaît pas.
    #[test]
    fn une_ecriture_preserve_les_autres_cles() {
        let (_tmp, cfg) = config_sur_disque();
        cfg.definir("cle_inconnue", json!("à garder")).unwrap();
        cfg.definir_langue(Langue::En).unwrap();
        cfg.definir_source_image(SourceImage::Yugipedia).unwrap();
        cfg.definir_grille_defaut(4, 4).unwrap();

        assert_eq!(cfg.brut("cle_inconnue"), Some(json!("à garder")));
        assert_eq!(cfg.langue(), Langue::En);
        assert_eq!(cfg.source_image(), SourceImage::Yugipedia);
    }

    /// La grille est écrite en **liste**, la forme que le Python produit —
    /// l'objet `{"cols", "rows"}` reste lu mais n'est plus écrit.
    #[test]
    fn la_grille_est_ecrite_en_liste() {
        let (_tmp, cfg) = config_sur_disque();
        cfg.definir_grille_defaut(4, 3).unwrap();
        assert_eq!(cfg.brut(CLE_GRILLE), Some(json!([4, 3])));
    }

    /// L'ancienne clé booléenne n'est pas touchée : la lecture ne la consulte
    /// que si la clé entière est absente, et elle ne l'est plus.
    #[test]
    fn l_ancienne_cle_booleenne_survit_a_l_ecriture_de_la_nouvelle() {
        let (_tmp, cfg) = config_sur_disque();
        cfg.definir(CLE_N_RARETES_LEGACY, json!(true)).unwrap();
        assert_eq!(cfg.n_raretes_par_artwork(), 1, "l'ancienne clé s'applique");

        cfg.definir_n_raretes_par_artwork(3).unwrap();
        assert_eq!(cfg.n_raretes_par_artwork(), 3, "la nouvelle prime");
        assert_eq!(
            cfg.brut(CLE_N_RARETES_LEGACY),
            Some(json!(true)),
            "l'ancienne est conservée telle quelle"
        );
    }

    /// Le fichier écrit doit être relisible par une **autre** instance : le
    /// cache mémoire ne doit pas masquer une écriture qui n'a pas atteint le
    /// disque.
    #[test]
    fn le_fichier_ecrit_se_relit_depuis_une_autre_instance() {
        let tmp = tempfile::tempdir().unwrap();
        let chemin = tmp.path().join("bdd").join("app_config.json");

        let premiere = Config::charger(&chemin);
        premiere.definir_grille_defaut(4, 4).unwrap();
        premiere.definir_langue(Langue::En).unwrap();

        let seconde = Config::charger(&chemin);
        assert_eq!(seconde.grille_defaut(), (4, 4));
        assert_eq!(seconde.langue(), Langue::En);
    }
}
