// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Artworks alternatifs — les images que Yugipedia héberge et qu'aucune API ne
//! décrit.
//!
//! Portage de `module/img_dl/yugipedia_artwork.py` (1 097 lignes).
//!
//! # Le problème
//!
//! Certains sets réimpriment une carte avec une **illustration différente**
//! sans créer de nouvelle entrée côté YGOPRODeck. Cas d'école, le *variant art
//! pool* de Rarity Collection 5 :
//!
//! ```text
//! RA05-EN134 ; Vanquish Soul Razen                                  (art standard)
//! RA05-EN134 ; Vanquish Soul Razen // description::(stamp artwork)  (la variante)
//! ```
//!
//! L'API ne renvoie qu'**une** entrée `card_images` pour cette carte. Le
//! classeur affiche donc l'illustration d'origine, et « Modifier l'artwork » ne
//! propose rien. Yugipedia, lui, héberge bien les deux fichiers.
//!
//! # La convention de nommage
//!
//! ```text
//! <NomCarteSansEspaces>-<PREFIXE_SET>-<LANGUE>-<RARETE>-<EDITION>[-<VARIANTE>].png
//!
//! VanquishSoulRazen-RA05-EN-UR-1E.png        une illustration du tirage
//! VanquishSoulRazen-RA05-EN-UR-1E-AA.png     la seconde, dite « AA »
//! RedEyesDarkDragoon-RA05-EN-UR-1E-EA.png    « EA », extended art
//! ```
//!
//! # Le piège : le suffixe ne dit pas « ceci est une variante »
//!
//! `-AA` et `-EA` ne sont que des **désambiguïsateurs**. Yugipedia ne les
//! ajoute que quand deux fichiers porteraient sinon le même nom. Vérifié le
//! 2026-08-24 : `RA05-EN083` Dark Magician *est* du stamp artwork, et ses
//! fichiers s'appellent `DarkMagician-RA05-EN-UR-1E.png` — sans suffixe, parce
//! qu'il n'existe qu'une illustration de ce tirage. À l'inverse `RA05-EN134` en
//! Ultra Rare a bien deux fichiers, d'où le `-AA` sur l'un.
//!
//! L'autorité est donc la page « Set Card Lists », qui sépare `== Main pool ==`
//! de `== Variant art pool ==` et annote chaque ligne — d'où
//! [`parser_variant_pool`]. Sur RA05 : main pool EN001→EN082, variant art pool
//! EN083→EN150.
//!
//! # Identifiants synthétiques
//!
//! Ces artworks n'ont **pas** de password Konami. Pour traverser le pipeline
//! existant — dédoublonnage, tri par artwork, cache disque, anti-doublon du
//! classeur — on leur attribue un identifiant **négatif** déterministe :
//! [`id_synthetique`], le CRC32 du nom de fichier. Les identifiants YGOPRODeck
//! étant toujours positifs, la distinction est sans ambiguïté (cf.
//! [`ygo_core::image_source::est_image_externe`]). L'URL n'est donc **jamais**
//! reconstruite depuis l'identifiant : elle est lue telle quelle.
//!
//! # Module interdit de paraphrase
//!
//! Comme le parser de Set lists, ce découpage encaisse des noms de fichiers
//! écrits à la main par des contributeurs. Chaque tolérance correspond à une
//! forme réellement rencontrée. L'oracle en couvre **1 613**, tirés de la table
//! `card_images_externes` de l'installation réelle.

use std::collections::{BTreeMap, BTreeSet};

use ygo_core::rarity::reference;

/// Codes langue rencontrés dans les noms de fichiers (`_LANGS`).
pub const LANGUES: [&str; 18] = [
    "EN", "FR", "DE", "IT", "PT", "SP", "ES", "JP", "JA", "KR", "KO", "AE", "SC", "TC", "EU", "NA",
    "AU", "OC",
];

/// Codes d'édition (`_EDITIONS`). Leur position dans le nom est variable.
pub const EDITIONS: [&str; 3] = ["1E", "UE", "LE"];

/// Jetons de contexte, ignorés à l'identification (`_CONTEXTES`).
pub const CONTEXTES: [&str; 5] = ["OP", "DT", "VG", "PR", "PROMO"];

/// Nom de rareté anglais → abréviation Yugipedia (`_RARETE_ABBR`).
///
/// Ne sert **qu'au classement** des candidats : une abréviation inconnue ne
/// filtre jamais un résultat.
pub const ABBR_RARETE: [(&str, &str); 34] = [
    ("common", "C"),
    ("short print", "SP"),
    ("super short print", "SSP"),
    ("normal parallel rare", "NPR"),
    ("rare", "R"),
    ("super rare", "SR"),
    ("ultra rare", "UR"),
    ("ultimate rare", "UtR"),
    ("secret rare", "ScR"),
    ("ultra secret rare", "UScR"),
    ("secret ultra rare", "ScUR"),
    ("prismatic secret rare", "PScR"),
    ("prismatic ultimate rare", "PUtR"),
    ("platinum secret rare", "PlScR"),
    ("platinum rare", "PlR"),
    ("starlight rare", "StR"),
    ("collector's rare", "CR"),
    ("collectors rare", "CR"),
    ("quarter century secret rare", "QCScR"),
    ("quarter century ultra rare", "QCUR"),
    ("ghost rare", "GR"),
    ("gold rare", "GUR"),
    ("gold secret rare", "GScR"),
    ("premium gold rare", "PGR"),
    ("parallel rare", "PR"),
    ("starfoil rare", "SFR"),
    ("mosaic rare", "MSR"),
    ("shatterfoil rare", "SHR"),
    ("extra secret rare", "EScR"),
    ("duel terminal normal parallel rare", "DNPR"),
    ("duel terminal rare parallel rare", "DRPR"),
    ("duel terminal super parallel rare", "DSPR"),
    ("duel terminal ultra parallel rare", "DUPR"),
    ("ultra rare (pharaoh's rare)", ""),
];

/// Libellés lisibles des familles de variante (`_LIBELLE_VARIANTE`).
const LIBELLE_VARIANTE: [(&str, &str); 3] = [
    ("AA", "Alternate Artwork"),
    ("EA", "Extended Art"),
    ("ALT", "Artwork alternatif"),
];

/// Annotation de Set list → famille de variante (`_ANNOTATION_VARIANTE`).
const ANNOTATION_VARIANTE: [(&str, &str); 6] = [
    ("stamp artwork", "AA"),
    ("alternate art", "AA"),
    ("alternate artwork", "AA"),
    ("new artwork", "AA"),
    ("extended art", "EA"),
    ("extended artwork", "EA"),
];

/// Titre de section signalant le pool de variantes (`_TITRE_VARIANT`).
const TITRE_VARIANT: &str = "variant art";

// ─────────────────────────────────────────────────────────────────────────────
// Clés et préfixes
// ─────────────────────────────────────────────────────────────────────────────

/// Clé de comparaison d'un nom : sans casse ni ponctuation.
///
/// Portage de `_cle` / `cle_comparaison`. `Ghost Ogre & Snow Rabbit` et
/// `GhostOgre&SnowRabbit` donnent la même clé — indispensable, Yugipedia
/// supprimant espaces et tirets mais conservant `&` et `:` de façon
/// irrégulière.
pub fn cle_comparaison(texte: &str) -> String {
    texte
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        .collect()
}

/// Préfixe de recherche sûr pour l'API `allimages`.
///
/// Portage de `_prefixe_recherche`. On s'arrête au premier caractère non
/// alphanumérique du nom compacté : ainsi aucune divergence de normalisation
/// (`&`, `:`, `'`, `.`…) ne peut faire manquer un fichier. Le filtrage exact
/// vient après, côté client.
///
/// | entrée | sortie |
/// |---|---|
/// | `Vanquish Soul Razen` | `VanquishSoulRazen` |
/// | `Red-Eyes Dark Dragoon` | `RedEyesDarkDragoon` |
/// | `Ghost Ogre & Snow Rabbit` | `GhostOgre` |
/// | `Number 99: Utopia Drag…` | `Number99` |
pub fn prefixe_recherche(nom_carte: &str) -> String {
    // `[\s\-‐-―]` : espaces, tiret ASCII, et les tirets Unicode U+2010..U+2015.
    let compact: String = nom_carte
        .trim()
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-' && !('\u{2010}'..='\u{2015}').contains(c))
        .collect();
    compact
        .chars()
        .take_while(char::is_ascii_alphanumeric)
        .take(24)
        .collect()
}

/// Préfixe de set d'un code : `RA05-EN134` → `RA05`.
pub fn prefixe_set(set_code: &str) -> String {
    set_code
        .split_once('-')
        .map_or(set_code, |(avant, _)| avant)
        .trim()
        .to_uppercase()
}

/// Langue d'un code de set : `RA05-FR134` → `FR`, `RA05` → `""`.
///
/// Rend la chaîne vide quand les deux lettres ne sont pas une langue connue —
/// c'est ce qui distingue un préfixe de langue d'un début de numéro.
pub fn langue_set_code(set_code: &str) -> String {
    let code = set_code.trim().to_uppercase();
    let Some((avant, apres)) = code.split_once('-') else {
        return String::new();
    };
    // `^[A-Z0-9]+-([A-Z]{2})` : le segment de gauche doit être alphanumérique.
    if avant.is_empty()
        || !avant
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
    {
        return String::new();
    }
    let langue: String = apres.chars().take(2).collect();
    if langue.len() == 2
        && langue.bytes().all(|b| b.is_ascii_uppercase())
        && LANGUES.contains(&langue.as_str())
    {
        langue
    } else {
        String::new()
    }
}

/// Abréviation Yugipedia d'une rareté : `Ultra Rare` → `UR`, `""` si inconnue.
///
/// Portage d'`abbr_rarete`. Le libellé passe d'abord par le référentiel
/// ([`reference::nom_vers_code`]) : c'est ce qui fait accepter le français, les
/// alias YGOPRODeck et n'importe quelle casse.
///
/// # Une entrée morte, conservée
///
/// `("short print", "SP")` de la table n'est **jamais atteinte** : le
/// référentiel range « Short Print » parmi les alias de `Common`, si bien que
/// le libellé anglais devient `Common` et l'abréviation `C`. Même chose pour
/// « Common Parallel Rare » qui ressort en `NPR`, et pour « Super Parallel
/// Rare » qui ressort en `DSPR`. La table est reproduite telle quelle, entrées
/// mortes comprises.
pub fn abbr_rarete(rarete_complete: &str) -> String {
    let nom = rarete_complete.trim();
    if nom.is_empty() {
        return String::new();
    }
    let nom_en = match reference::nom_vers_code(nom) {
        Some(code) => reference::code_vers_nom_en(code),
        None => nom.to_owned(),
    };
    let cle = nom_en.trim().to_lowercase();
    ABBR_RARETE
        .iter()
        .find(|(k, _)| *k == cle)
        .map_or(String::new(), |(_, v)| (*v).to_owned())
}

/// Identifiant déterministe et **négatif** d'un fichier Yugipedia.
///
/// Portage d'`id_synthetique`. Négatif, donc jamais confondu avec un password
/// Konami — ils sont toujours positifs. Déterministe, donc le cache disque
/// `img/small/<id>.jpg` reste valide d'une session à l'autre.
pub fn id_synthetique(fichier: &str) -> i64 {
    let crc = i64::from(crc32fast::hash(fichier.as_bytes()));
    -((crc % 2_000_000_000) + 1)
}

// ─────────────────────────────────────────────────────────────────────────────
// Découpage d'un nom de fichier
// ─────────────────────────────────────────────────────────────────────────────

/// Ce qu'un nom de fichier Yugipedia dit du tirage qu'il illustre.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InfosFichier {
    /// Nom de fichier complet, extension comprise.
    pub fichier: String,
    /// Code langue, `""` si absent.
    pub langue: String,
    /// Abréviation de rareté, **casse d'origine conservée** (`PScR`, `StR`…).
    pub rarete_abbr: String,
    /// Code d'édition (`1E`, `UE`, `LE`), `""` si absent.
    pub edition: String,
    /// Suffixe de variante (`AA`, `EA`, `ALT`, éventuellement numéroté).
    pub variante: String,
}

/// Retire l'extension, à la manière de `os.path.splitext`.
///
/// Les points de tête ne comptent pas comme extension : `.png` reste `.png`.
fn sans_extension(nom: &str) -> &str {
    let debut = nom.len() - nom.trim_start_matches('.').len();
    match nom.get(debut..).and_then(|reste| reste.rfind('.')) {
        Some(i) => nom.get(..debut + i).unwrap_or(nom),
        None => nom,
    }
}

/// Le jeton est-il un suffixe de variante ? (`^(AA|EA|ALT)(\d*)$`)
///
/// Rend `(famille, chiffres)` — `AA2` donne `("AA", "2")`.
fn suffixe_variante(jeton: &str) -> Option<(&'static str, &str)> {
    let majuscule = jeton.to_ascii_uppercase();
    // Aucune des trois familles n'est le préfixe d'une autre : l'ordre du
    // parcours est sans effet, contrairement à ce qu'on pourrait craindre.
    for famille in ["ALT", "AA", "EA"] {
        if let Some(reste) = majuscule.strip_prefix(famille) {
            if reste.bytes().all(|b| b.is_ascii_digit()) {
                let debut = famille.len();
                return jeton.get(debut..).map(|chiffres| (famille, chiffres));
            }
        }
    }
    None
}

/// Découpe un nom de fichier autour du préfixe de set.
///
/// Portage de `_decouper_fichier`. Rend `(segment avant le préfixe, infos)`, ou
/// `None` si le fichier ne concerne pas ce set — ou n'a **rien devant** le
/// préfixe, ce qui écarte les covers comme `RA05-BoosterEN.png`.
///
/// C'est le socle commun d'[`analyser_fichier`], qui **valide** un nom de carte
/// attendu, et d'[`analyser_fichier_set`], qui l'**extrait** — la carte n'étant
/// pas connue d'avance lors d'une passe par set.
///
/// # Écart assumé
///
/// Le Python cherche le marqueur dans `base.upper()` tout en découpant `base` :
/// une mise en majuscules qui change la longueur décalerait les indices. La
/// recherche est ici insensible à la casse **ASCII** sur la chaîne d'origine —
/// même résultat sur tout nom de fichier réel, sans le décalage possible.
pub fn decouper_fichier(nom_fichier: &str, prefixe_set: &str) -> Option<(String, InfosFichier)> {
    let base = sans_extension(nom_fichier);
    let marqueur = format!("-{prefixe_set}-");
    let idx = position_ascii_insensible(base.as_bytes(), marqueur.as_bytes())?;
    if idx == 0 {
        return None; // rien devant le préfixe : ce n'est pas une carte.
    }

    let apres = base.get(idx + marqueur.len()..)?;
    let jetons: Vec<&str> = apres.split('-').filter(|t| !t.is_empty()).collect();
    let premier = jetons.first()?;

    let langue = if LANGUES.contains(&premier.to_ascii_uppercase().as_str()) {
        premier.to_ascii_uppercase()
    } else {
        String::new()
    };
    let reste = if langue.is_empty() {
        &jetons[..]
    } else {
        jetons.get(1..).unwrap_or(&[])
    };

    let (mut variante, mut edition, mut rarete) = (String::new(), String::new(), String::new());
    for jeton in reste {
        let majuscule = jeton.to_ascii_uppercase();
        if let Some((famille, chiffres)) = suffixe_variante(jeton) {
            variante = format!("{famille}{chiffres}");
            continue;
        }
        if EDITIONS.contains(&majuscule.as_str()) {
            edition = majuscule;
            continue;
        }
        if CONTEXTES.contains(&majuscule.as_str()) {
            continue;
        }
        if rarete.is_empty() {
            // Le jeton d'origine, casse comprise : `PScR` et `StR` en dépendent.
            rarete = (*jeton).to_owned();
        }
    }

    Some((
        base.get(..idx)?.to_owned(),
        InfosFichier {
            fichier: nom_fichier.to_owned(),
            langue,
            rarete_abbr: rarete,
            edition,
            variante,
        },
    ))
}

fn position_ascii_insensible(foin: &[u8], aiguille: &[u8]) -> Option<usize> {
    if aiguille.is_empty() || foin.len() < aiguille.len() {
        return None;
    }
    (0..=foin.len() - aiguille.len()).find(|&i| {
        foin.get(i..i + aiguille.len())
            .is_some_and(|f| f.eq_ignore_ascii_case(aiguille))
    })
}

/// Découpe un fichier **en validant** qu'il concerne bien cette carte.
///
/// Portage d'`_analyser_fichier`.
pub fn analyser_fichier(
    nom_fichier: &str,
    nom_carte: &str,
    prefixe_set: &str,
) -> Option<InfosFichier> {
    let (segment, infos) = decouper_fichier(nom_fichier, prefixe_set)?;
    (cle_comparaison(&segment) == cle_comparaison(nom_carte)).then_some(infos)
}

/// Découpe un fichier **en extrayant** le nom de carte.
///
/// Portage d'`_analyser_fichier_set`, employé par la passe par set où l'on
/// découvre les cartes au lieu de les connaître. Rend `(segment brut, clé de
/// comparaison, infos)`.
pub fn analyser_fichier_set(
    nom_fichier: &str,
    prefixe_set: &str,
) -> Option<(String, String, InfosFichier)> {
    let (segment, infos) = decouper_fichier(nom_fichier, prefixe_set)?;
    let cle = cle_comparaison(&segment);
    (!cle.is_empty()).then_some((segment, cle, infos))
}

/// Un fichier de set, prêt à être apparié à un slot de la structure.
///
/// C'est ce que le Python appelle « infos » une fois passé dans
/// `_indexer_fichiers_set` : le découpage du nom, plus les trois champs que
/// l'indexation ajoute — l'URL, l'identifiant synthétique et l'uuid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidat {
    /// Le nom de carte tel qu'il apparaît dans le nom de fichier.
    pub segment: String,
    /// Clé de comparaison de ce nom.
    pub cle_carte: String,
    /// Le découpage du nom de fichier.
    pub infos: InfosFichier,
    /// URL pleine résolution.
    pub card_url: String,
    /// Identifiant synthétique, négatif (cf. [`id_synthetique`]).
    pub image_id: i64,
    /// `yugipedia:<fichier>`.
    pub uuid: String,
}

/// Index des fichiers d'un set, par `(clé de carte, rareté en MAJUSCULES)`.
pub type IndexFichiers = std::collections::BTreeMap<(String, String), Vec<Candidat>>;

/// Indexe une liste de fichiers déjà ramenés.
///
/// Portage d'`_indexer_fichiers_set`, **sans son appel réseau** — c'est ce
/// découpage qui rend l'indexation éprouvable sur les 302 fichiers `LOCR`
/// capturés, sans une requête.
///
/// Deux filtres, dans cet ordre :
///
/// 1. la **langue** — un fichier qui en déclare une doit déclarer la bonne ;
///    un fichier qui n'en déclare aucune passe ;
/// 2. le **classeur** — si `noms_cartes` n'est pas vide, un fichier dont la
///    carte n'y figure pas est écarté.
///
/// Contrairement au tri « par suffixe », **tous** les fichiers de carte sont
/// gardés, suffixés ou non : le suffixe ne dit pas si le tirage est une
/// variante. C'est la Set list qui le dit.
pub fn indexer_fichiers_set(
    fichiers: &[FichierDistant],
    prefixe_set: &str,
    langue: &str,
    noms_cartes: &[String],
) -> IndexFichiers {
    let cles_classeur: std::collections::HashSet<String> =
        noms_cartes.iter().map(|n| cle_comparaison(n)).collect();

    let mut index: IndexFichiers = std::collections::BTreeMap::new();
    for distant in fichiers {
        let Some((segment, cle, infos)) = analyser_fichier_set(&distant.nom, prefixe_set) else {
            continue;
        };
        if !infos.langue.is_empty() && infos.langue != langue {
            continue;
        }
        if !cles_classeur.is_empty() && !cles_classeur.contains(&cle) {
            continue;
        }
        let rarete = infos.rarete_abbr.to_ascii_uppercase();
        index
            .entry((cle.clone(), rarete))
            .or_default()
            .push(Candidat {
                segment,
                cle_carte: cle,
                image_id: id_synthetique(&distant.nom),
                uuid: format!("yugipedia:{}", distant.nom),
                card_url: distant.url.clone(),
                infos,
            });
    }
    index
}

/// L'illustration telle que l'application la stocke.
///
/// Portage de `_formater_artwork`. `badge` et `libelle` ne servent qu'à
/// l'affichage ; les quatre premiers champs sont ceux que la passe écrit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artwork {
    /// `yugipedia:<fichier>`.
    pub card_image_uuid: String,
    /// Identifiant synthétique, négatif.
    pub card_image_id: i64,
    /// URL pleine résolution.
    pub card_image_url: String,
    /// URL de vignette — **la même** sur ce chemin, faute d'`art_url`.
    pub card_image_small: String,
    /// Étiquette courte, pour un bouton radio.
    pub badge: String,
    /// Étiquette lisible, pour une ligne de liste.
    pub libelle: String,
}

/// Candidat interne → illustration au format de l'application.
///
/// # Un champ que ce chemin ne renseigne jamais
///
/// Le Python lit `art_url` et retombe sur `card_url` s'il est absent.
/// `_indexer_fichiers_set` ne pose jamais `art_url` : sur ce chemin,
/// `card_image_small` **est** `card_image_url`. Les 30 lignes de l'oracle
/// `LOCR-JP` le confirment, sans exception.
pub fn formater_artwork(candidat: &Candidat, prefixe_set: &str) -> Artwork {
    let variante = candidat.infos.variante.to_ascii_uppercase();
    let racine = variante.trim_end_matches(|c: char| c.is_ascii_digit());
    let libelle_variante = LIBELLE_VARIANTE
        .iter()
        .find(|(cle, _)| *cle == racine)
        .map_or("Print Yugipedia", |(_, libelle)| *libelle);

    let details: Vec<&str> = [
        prefixe_set,
        &candidat.infos.langue,
        &candidat.infos.rarete_abbr,
        &candidat.infos.edition,
    ]
    .into_iter()
    .filter(|p| !p.is_empty())
    .collect();
    let details = details.join(" · ");

    Artwork {
        card_image_uuid: candidat.uuid.clone(),
        card_image_id: candidat.image_id,
        card_image_url: candidat.card_url.clone(),
        card_image_small: candidat.card_url.clone(),
        badge: if variante.is_empty() {
            "Print".to_owned()
        } else {
            variante.clone()
        },
        libelle: if details.is_empty() {
            libelle_variante.to_owned()
        } else {
            format!("{libelle_variante} ({details})")
        },
    }
}

/// Le fichier porte-t-il un suffixe de la famille « extended art » ?
pub fn est_extended_art(infos: &InfosFichier) -> bool {
    infos.variante.to_ascii_uppercase().starts_with("EA")
}

// ─────────────────────────────────────────────────────────────────────────────
// Étiquettes
// ─────────────────────────────────────────────────────────────────────────────

/// Badge court d'un artwork externe, déduit de son `card_image_uuid`.
///
/// Portage de `badge_depuis_uuid`. Rend `""` pour un uuid qui n'est pas un
/// artwork externe, et `"Print"` pour un fichier sans suffixe de variante.
/// Permet d'étiqueter un artwork relu depuis `card_images`, où seul l'uuid
/// subsiste.
pub fn badge_depuis_uuid(card_image_uuid: &str) -> String {
    let Some(fichier) = card_image_uuid.strip_prefix("yugipedia:") else {
        return String::new();
    };
    let base = sans_extension(fichier);
    for jeton in base.split('-').rev() {
        if let Some((famille, chiffres)) = suffixe_variante(jeton) {
            return format!("{famille}{chiffres}");
        }
    }
    "Print".to_owned()
}

/// Libellé lisible d'un artwork externe, `""` si l'uuid n'en est pas un.
pub fn libelle_depuis_uuid(card_image_uuid: &str) -> String {
    let badge = badge_depuis_uuid(card_image_uuid);
    if badge.is_empty() {
        return String::new();
    }
    let racine = badge.trim_end_matches(|c: char| c.is_ascii_digit());
    let nom = LIBELLE_VARIANTE
        .iter()
        .find(|(k, _)| *k == racine)
        .map_or("Print Yugipedia", |(_, v)| v);
    let fichier = card_image_uuid
        .split_once(':')
        .map_or(card_image_uuid, |(_, apres)| apres);
    format!("{nom} — Yugipedia ({fichier})")
}

// ─────────────────────────────────────────────────────────────────────────────
// Classement
// ─────────────────────────────────────────────────────────────────────────────

/// Classe un candidat : langue du set d'abord, puis rareté du slot, puis
/// première édition.
///
/// Portage de `_score`. Les poids sont ceux du Python, y compris le `+8` qui
/// place `AA` — variante d'illustration franche — avant `EA`, qui n'est que le
/// même dessin dans un cadre étendu.
pub fn score(infos: &InfosFichier, langue_cible: &str, abbr_cible: &str) -> i64 {
    let mut s = 0;
    let langue = infos.langue.as_str();
    if !langue.is_empty() && !langue_cible.is_empty() && langue == langue_cible {
        s += 60;
    } else if langue == "EN" {
        s += 30;
    } else if !langue.is_empty() {
        s += 10;
    }
    if !abbr_cible.is_empty() && infos.rarete_abbr.eq_ignore_ascii_case(abbr_cible) {
        s += 40;
    }
    if infos.edition == "1E" {
        s += 5;
    }
    if infos.variante.to_ascii_uppercase().starts_with("AA") {
        s += 8;
    }
    s
}

// ─────────────────────────────────────────────────────────────────────────────
// Variant art pool
// ─────────────────────────────────────────────────────────────────────────────

/// Codes de set du « variant art pool » d'une Set list → famille de variante.
///
/// Portage de `parser_variant_pool`. Un code est retenu si **sa ligne** porte
/// une annotation `// description::(stamp artwork)` ou `(extended art)`, **ou**
/// si elle se trouve sous une section dont le titre contient « variant art ».
/// Les deux signaux sont cumulés : certaines pages n'ont que la section,
/// d'autres que les annotations.
///
/// Une annotation inconnue qui parle d'illustration — elle contient « art » —
/// vaut `AA` plutôt que rien : le libellé exact varie d'un set à l'autre, et
/// rater une variante coûte plus cher qu'en supposer une.
pub fn parser_variant_pool(wikitext: &str, prefixe_set: &str) -> BTreeMap<String, String> {
    let mut sortie = BTreeMap::new();
    for ligne in lignes_de_set_list(wikitext, prefixe_set) {
        if !ligne.famille.is_empty() {
            sortie.insert(ligne.code, ligne.famille);
        }
    }
    sortie
}

/// Les numéros qu'une Set list annonce imprimés en **illustration
/// alternative**.
///
/// Deux signaux, l'un ou l'autre suffit :
///
/// - la famille `AA` de [`parser_variant_pool`] — `(alternate art)`,
///   `(new artwork)`, `(stamp artwork)`, `(6th artwork)`, `(Arkana)`… ou une
///   ligne sous une section « variant art » ;
/// - la **colonne d'impression** — quatrième champ de la ligne — qui dit
///   `New artwork`. C'est la forme des Set lists OCG : `LOCH-JP027; I:P
///   Masquerena; Ultra Rare, …; New artwork`, sans annotation `description`.
///   C'est aussi de là que viennent les fausses raretés « New » et « New
///   artwork » d'YGOPRODeck.
///
/// Un numéro qui a **plusieurs** lignes — `LOCH-JP001`, une normale et une en
/// `(extended art)` — est retenu si l'une d'elles est alternative : à la
/// différence de [`parser_variant_pool`], la dernière ligne n'efface pas les
/// précédentes. `(extended art)` seul ne compte pas : c'est la même
/// illustration, cadre élargi.
///
/// ```
/// use ygo_sources::yugipedia::artwork::numeros_illustration_alternative;
/// let w = "LOCH-JP026; W:P Fancy Ball; Ultra Rare; New\n\
///          LOCH-JP027; I:P Masquerena; Ultra Rare; New artwork\n\
///          LOCH-JP001; DM; Ultra Rare; New // description::(extended art)\n";
/// let alt = numeros_illustration_alternative(w, "LOCH");
/// assert_eq!(alt.into_iter().collect::<Vec<_>>(), ["LOCH-JP027"]);
/// ```
#[must_use]
pub fn numeros_illustration_alternative(wikitext: &str, prefixe_set: &str) -> BTreeSet<String> {
    lignes_de_set_list(wikitext, prefixe_set)
        .into_iter()
        .filter(|l| l.famille == "AA" || l.impression.to_lowercase().contains("artwork"))
        .map(|l| l.code)
        .collect()
}

/// Tous les numéros qu'une Set list mentionne pour ce set.
///
/// Sert à distinguer « la page ne dit rien de ce numéro » — il n'y figure
/// pas, et l'on ne peut rien conclure — de « la page le liste sans illustration
/// alternative ».
#[must_use]
pub fn numeros_de_set_list(wikitext: &str, prefixe_set: &str) -> BTreeSet<String> {
    lignes_de_set_list(wikitext, prefixe_set)
        .into_iter()
        .map(|l| l.code)
        .collect()
}

/// Une ligne de Set list, réduite à ce que les lectures ci-dessus en tirent.
struct LigneSetList {
    code: String,
    /// `AA`, `EA` ou vide.
    famille: String,
    /// La colonne d'impression (`New`, `New artwork`, `Reprint`…), vide si
    /// la ligne n'en a pas.
    impression: String,
}

/// Lit les lignes d'une Set list qui commencent par un code du set.
fn lignes_de_set_list(wikitext: &str, prefixe_set: &str) -> Vec<LigneSetList> {
    let mut sortie = Vec::new();
    if wikitext.is_empty() || prefixe_set.is_empty() {
        return sortie;
    }
    let prefixe = prefixe_set.to_uppercase();
    let mut section_variante = false;

    for brut in wikitext.lines() {
        let ligne = brut.trim();

        if let Some(titre) = titre_de_section(ligne) {
            section_variante = titre.to_lowercase().contains(TITRE_VARIANT);
            continue;
        }

        let ligne = ligne.trim_start_matches('|').trim();
        if ligne.is_empty() || !commence_par_code(ligne, &prefixe) {
            continue;
        }
        let code = ligne
            .split_once(';')
            .map_or(ligne, |(avant, _)| avant)
            .trim()
            .to_uppercase();
        if code.is_empty() {
            continue;
        }

        let famille = match annotation_description(ligne) {
            Some(annotation) => {
                let annotation = annotation.trim().to_lowercase();
                ANNOTATION_VARIANTE
                    .iter()
                    .find(|(k, _)| *k == annotation)
                    .map(|(_, v)| (*v).to_owned())
                    .or_else(|| annotation.contains("art").then(|| "AA".to_owned()))
                    .unwrap_or_default()
            }
            None if section_variante => "AA".to_owned(),
            None => String::new(),
        };
        let sans_note = ligne.split_once("//").map_or(ligne, |(avant, _)| avant);
        let impression = sans_note
            .split(';')
            .nth(3)
            .map(str::trim)
            .unwrap_or_default()
            .to_owned();

        sortie.push(LigneSetList {
            code,
            famille,
            impression,
        });
    }
    sortie
}

/// `^\s*=+\s*(.+?)\s*=+\s*$` — le titre d'une section wiki.
fn titre_de_section(ligne: &str) -> Option<&str> {
    let sans_gauche = ligne.trim_start_matches('=');
    if sans_gauche.len() == ligne.len() {
        return None; // pas de `=` de tête
    }
    let sans_droite = sans_gauche.trim_end_matches('=');
    if sans_droite.len() == sans_gauche.len() {
        return None; // pas de `=` de queue
    }
    let titre = sans_droite.trim();
    (!titre.is_empty()).then_some(titre)
}

/// `^<PREFIXE>-[A-Z]{0,3}[A-Z0-9]+`, insensible à la casse.
fn commence_par_code(ligne: &str, prefixe: &str) -> bool {
    let Some(reste) = ligne
        .get(..prefixe.len())
        .filter(|debut| debut.eq_ignore_ascii_case(prefixe))
        .and_then(|_| ligne.get(prefixe.len()..))
    else {
        return false;
    };
    let Some(apres_tiret) = reste.strip_prefix('-') else {
        return false;
    };
    // `[A-Z]{0,3}` est gourmand mais peut se rétracter ; ce qui suit doit être
    // au moins un caractère alphanumérique. En pratique : au moins un
    // caractère `[A-Za-z0-9]` après le tiret.
    apres_tiret
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric())
}

/// `description::\(([^)]*)\)`, insensible à la casse.
fn annotation_description(ligne: &str) -> Option<&str> {
    let minuscule = ligne.to_lowercase();
    let debut = minuscule.find("description::(")? + "description::(".len();
    let reste = ligne.get(debut..)?;
    let fin = reste.find(')')?;
    reste.get(..fin)
}

// ─────────────────────────────────────────────────────────────────────────────
// Réseau
// ─────────────────────────────────────────────────────────────────────────────

/// Nombre maximal de fichiers ramenés par requête `allimages` (`_AILIMIT`).
pub const LIMITE_ALLIMAGES: u32 = 200;

/// Taille de page de la recherche par set (`_GSR_LIMIT`).
pub const LIMITE_RECHERCHE_SET: u32 = 50;

/// Plafond de pages pour une passe par set (`_MAX_PAGES_SET`).
///
/// Mille fichiers par set. Le plafond n'est pas là pour économiser des
/// requêtes mais pour qu'une recherche qui part en vrille s'arrête : il est
/// **journalisé** quand il est atteint, plutôt que de tronquer en silence.
pub const MAX_PAGES_SET: usize = 20;

/// Un fichier Yugipedia : son nom et l'URL de l'image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FichierDistant {
    /// Nom du fichier, `VanquishSoulRazen-RA05-EN-UR-1E.png`.
    pub nom: String,
    /// URL pleine résolution.
    pub url: String,
}

#[derive(serde::Deserialize)]
struct ReponseAllimages {
    query: RequeteAllimages,
}

#[derive(serde::Deserialize)]
struct RequeteAllimages {
    #[serde(default)]
    allimages: Vec<ImageDistante>,
}

#[derive(serde::Deserialize)]
struct ImageDistante {
    #[serde(default)]
    name: String,
    #[serde(default)]
    url: String,
}

#[derive(serde::Deserialize)]
struct ReponseRecherche {
    #[serde(default)]
    query: Option<RequeteRecherche>,
    #[serde(rename = "continue", default)]
    suite: Option<Suite>,
}

#[derive(serde::Deserialize)]
struct RequeteRecherche {
    #[serde(default)]
    pages: Vec<PageRecherche>,
}

#[derive(serde::Deserialize)]
struct PageRecherche {
    #[serde(default)]
    title: String,
    #[serde(default)]
    imageinfo: Vec<InfoImage>,
}

#[derive(serde::Deserialize)]
struct InfoImage {
    #[serde(default)]
    url: String,
}

#[derive(serde::Deserialize)]
struct Suite {
    #[serde(default)]
    gsroffset: Option<i64>,
}

/// Tous les fichiers dont le nom commence par `prefixe`.
///
/// Portage de `_lister_images`. `allimages` ne sait filtrer que sur un préfixe,
/// donc sur le **nom de carte** — d'où [`prefixe_recherche`], qui s'arrête au
/// premier caractère qu'une normalisation pourrait faire diverger.
pub async fn lister_images(
    client: &crate::http::ClientHttp,
    prefixe: &str,
) -> crate::error::Result<Vec<FichierDistant>> {
    if prefixe.is_empty() {
        return Ok(Vec::new());
    }
    let url = super::url_api(&[
        ("action", "query"),
        ("list", "allimages"),
        ("aiprefix", prefixe),
        ("ailimit", &LIMITE_ALLIMAGES.to_string()),
        ("aiprop", "url"),
    ]);
    let reponse: ReponseAllimages = client.get_json(&url).await?;
    Ok(reponse
        .query
        .allimages
        .into_iter()
        .filter(|i| !i.name.is_empty() && !i.url.is_empty())
        .map(|i| FichierDistant {
            nom: i.name,
            url: i.url,
        })
        .collect())
}

/// Tous les fichiers rattachés à un set.
///
/// Portage de `_rechercher_fichiers_set`. Passe par `generator=search` sur
/// l'espace de noms « File » avec `prop=imageinfo` : une requête ramène titres
/// **et** URLs. Mesuré sur RA05 : 166 fichiers en 4 requêtes, contre 150 —
/// une par carte — avec `allimages`.
///
/// La recherche est plein texte, donc volontairement large : elle remonte aussi
/// les covers et les tapis de jeu. Le tri vient après, par
/// [`analyser_fichier_set`], qui écarte tout ce qui n'a pas un nom de carte
/// devant le préfixe.
pub async fn rechercher_fichiers_set(
    client: &crate::http::ClientHttp,
    prefixe_set: &str,
    langue: &str,
) -> crate::error::Result<Vec<FichierDistant>> {
    if prefixe_set.is_empty() {
        return Ok(Vec::new());
    }
    let recherche = if langue.is_empty() {
        prefixe_set.to_owned()
    } else {
        format!("{prefixe_set}-{langue}")
    };

    let mut sortie: Vec<FichierDistant> = Vec::new();
    let mut vus: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut offset: Option<i64> = None;
    let mut pages = 0usize;
    let mut reste_des_pages = false;

    while pages < MAX_PAGES_SET {
        let mut parametres: Vec<(&str, String)> = vec![
            ("action", "query".to_owned()),
            ("generator", "search".to_owned()),
            ("gsrsearch", recherche.clone()),
            ("gsrnamespace", "6".to_owned()),
            ("gsrlimit", LIMITE_RECHERCHE_SET.to_string()),
            ("prop", "imageinfo".to_owned()),
            ("iiprop", "url".to_owned()),
        ];
        if let Some(o) = offset {
            parametres.push(("gsroffset", o.to_string()));
        }
        let refs: Vec<(&str, &str)> = parametres.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let url = super::url_api(&refs);

        let reponse: ReponseRecherche = client.get_json(&url).await?;
        for page in reponse.query.map(|q| q.pages).unwrap_or_default() {
            // Un titre de fichier s'écrit « File:Nom.png » : sans deux-points,
            // ce n'en est pas un.
            let Some((_, nom)) = page.title.split_once(':') else {
                continue;
            };
            let url_image = page
                .imageinfo
                .first()
                .map(|i| i.url.clone())
                .unwrap_or_default();
            if nom.is_empty() || url_image.is_empty() || !vus.insert(nom.to_owned()) {
                continue;
            }
            sortie.push(FichierDistant {
                nom: nom.to_owned(),
                url: url_image,
            });
        }

        pages += 1;
        match reponse.suite.and_then(|s| s.gsroffset) {
            Some(o) => {
                offset = Some(o);
                reste_des_pages = true;
            }
            None => {
                reste_des_pages = false;
                break;
            }
        }
    }

    if pages >= MAX_PAGES_SET && reste_des_pages {
        tracing::info!(
            set = prefixe_set,
            pages = MAX_PAGES_SET,
            fichiers = sortie.len(),
            "plafond de pages atteint — la liste peut être incomplète, \
             les artworks manquants restent accessibles au cas par cas"
        );
    }
    Ok(sortie)
}

/// Index des fichiers Yugipedia d'un set, prêt pour la passe artworks.
///
/// Portage de `fichiers_pour_set`. Une requête réseau, puis
/// [`indexer_fichiers_set`] — qui porte toute la logique et se teste sans elle.
///
/// # Deux écarts assumés, tous deux dans le même sens
///
/// Le Python fait deux choses de plus, et aucune ne concerne la passe :
///
/// - il **alimente un cache SQLite** par carte, pour servir le clic droit de
///   l'interface. Ce cache appartient au futur `ygo-images` ; l'écrire ici
///   violerait la règle R3 (un seul écrivain par base) ;
/// - il **avale toute exception** et rend un index vide. Ici l'erreur remonte :
///   c'est à l'appelant de décider qu'une panne réseau n'est pas fatale, et il
///   le fait explicitement. Un index vide et une panne réseau ne veulent pas
///   dire la même chose, et les confondre est exactement ce qui rend une passe
///   silencieusement inopérante.
pub async fn fichiers_pour_set(
    client: &crate::http::ClientHttp,
    set_code: &str,
    noms_cartes: &[String],
    langue: &str,
) -> crate::error::Result<IndexFichiers> {
    let prefixe = prefixe_set(set_code);
    if prefixe.is_empty() {
        return Ok(IndexFichiers::new());
    }
    let langue_cible = match (langue.is_empty(), langue_set_code(set_code)) {
        (false, _) => langue.to_ascii_uppercase(),
        (true, lg) if !lg.is_empty() => lg.to_ascii_uppercase(),
        _ => "EN".to_owned(),
    };

    let distants = rechercher_fichiers_set(client, &prefixe, &langue_cible).await?;
    let index = indexer_fichiers_set(&distants, &prefixe, &langue_cible, noms_cartes);
    tracing::info!(
        set = %prefixe,
        fichiers = index.values().map(Vec::len).sum::<usize>(),
        cartes = index.keys().map(|(cle, _)| cle).collect::<std::collections::BTreeSet<_>>().len(),
        "fichiers utilisables"
    );
    Ok(index)
}

/// Wikitext brut d'une page, redirections suivies.
///
/// Portage de `_fetch_wikitext`. Sert à lire la Set list d'un set pour en tirer
/// le *variant art pool* ([`parser_variant_pool`]).
pub async fn wikitext_page(
    client: &crate::http::ClientHttp,
    titre: &str,
) -> crate::error::Result<Option<String>> {
    #[derive(serde::Deserialize)]
    struct Reponse {
        #[serde(default)]
        parse: Option<Contenu>,
    }
    #[derive(serde::Deserialize)]
    struct Contenu {
        #[serde(default)]
        wikitext: String,
    }
    let url = super::url_api(&[
        ("action", "parse"),
        ("page", titre),
        ("prop", "wikitext"),
        ("redirects", "1"),
    ]);
    let reponse: Reponse = client.get_json(&url).await?;
    Ok(reponse.parse.map(|c| c.wikitext).filter(|w| !w.is_empty()))
}

/// Titres candidats de la page « Set Card Lists » d'un set.
///
/// Portage de `_titres_set_list`. L'ordre est celui d'un essai : la page TCG de
/// la langue demandée, puis la TCG anglaise en repli, puis l'OCG.
pub fn titres_set_list(nom_set: &str, langue: &str) -> Vec<String> {
    if nom_set.is_empty() {
        return Vec::new();
    }
    let lg = if langue.is_empty() {
        "EN".to_owned()
    } else {
        langue.to_uppercase()
    };
    let mut titres = vec![format!("Set Card Lists:{nom_set} (TCG-{lg})")];
    if lg != "EN" {
        titres.push(format!("Set Card Lists:{nom_set} (TCG-EN)"));
    }
    titres.push(format!("Set Card Lists:{nom_set} (OCG-{lg})"));
    titres
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::*;

    #[test]
    fn la_cle_ignore_casse_et_ponctuation() {
        assert_eq!(
            cle_comparaison("Ghost Ogre & Snow Rabbit"),
            "ghostogresnowrabbit"
        );
        assert_eq!(
            cle_comparaison("GhostOgre&SnowRabbit"),
            "ghostogresnowrabbit"
        );
        assert_eq!(cle_comparaison("Number 99: Utopia"), "number99utopia");
        assert_eq!(cle_comparaison(""), "");
    }

    #[test]
    fn le_prefixe_de_recherche_s_arrete_au_premier_caractere_exotique() {
        assert_eq!(
            prefixe_recherche("Vanquish Soul Razen"),
            "VanquishSoulRazen"
        );
        assert_eq!(
            prefixe_recherche("Red-Eyes Dark Dragoon"),
            "RedEyesDarkDragoon"
        );
        assert_eq!(prefixe_recherche("Ghost Ogre & Snow Rabbit"), "GhostOgre");
        assert_eq!(prefixe_recherche("Number 99: Utopia Dragonar"), "Number99");
        assert_eq!(prefixe_recherche(""), "");
        // Un nom qui commence par autre chose qu'une lettre ne donne rien.
        assert_eq!(prefixe_recherche("&Foo"), "");
        // Plafonné à 24 caractères.
        assert_eq!(prefixe_recherche(&"A".repeat(40)).len(), 24);
    }

    #[test]
    fn prefixe_et_langue_d_un_code_de_set() {
        assert_eq!(prefixe_set("RA05-EN134"), "RA05");
        assert_eq!(prefixe_set("LOCR-JP001"), "LOCR");
        assert_eq!(prefixe_set("RA05"), "RA05");
        assert_eq!(prefixe_set(""), "");
        assert_eq!(langue_set_code("RA05-FR134"), "FR");
        assert_eq!(langue_set_code("RA05-EN134"), "EN");
        assert_eq!(langue_set_code("RA05"), "");
        // Deux lettres qui ne sont pas une langue connue : rien.
        assert_eq!(langue_set_code("RA05-XX134"), "");
    }

    #[test]
    fn l_identifiant_est_negatif_et_deterministe() {
        let a = id_synthetique("VanquishSoulRazen-RA05-EN-UR-1E.png");
        assert!(a < 0);
        assert_eq!(a, id_synthetique("VanquishSoulRazen-RA05-EN-UR-1E.png"));
        assert_ne!(a, id_synthetique("VanquishSoulRazen-RA05-EN-UR-1E-AA.png"));
        // Toujours dans les bornes, jamais zéro.
        assert!((-2_000_000_000..=-1).contains(&a));
        assert_eq!(id_synthetique(""), -1);
    }

    #[test]
    fn decoupage_d_un_nom_de_fichier_complet() {
        let (segment, infos) =
            decouper_fichier("VanquishSoulRazen-RA05-EN-UR-1E-AA.png", "RA05").unwrap();
        assert_eq!(segment, "VanquishSoulRazen");
        assert_eq!(infos.langue, "EN");
        assert_eq!(infos.rarete_abbr, "UR");
        assert_eq!(infos.edition, "1E");
        assert_eq!(infos.variante, "AA");
    }

    #[test]
    fn la_casse_de_la_rarete_est_conservee() {
        // `PScR`, `StR`, `PlScR` : la casse fait partie du libellé Yugipedia.
        let (_, infos) = decouper_fichier("Carte-LOCR-JP-PScR-1E.png", "LOCR").unwrap();
        assert_eq!(infos.rarete_abbr, "PScR");
    }

    #[test]
    fn une_cover_sans_nom_de_carte_est_ecartee() {
        // `RA05-BoosterEN.png` : rien devant le préfixe.
        assert!(decouper_fichier("RA05-BoosterEN.png", "RA05").is_none());
        // Fichier d'un autre set.
        assert!(decouper_fichier("Carte-RA02-EN-UR-1E.png", "RA05").is_none());
    }

    #[test]
    fn les_jetons_de_contexte_ne_deviennent_pas_des_raretes() {
        let (_, infos) = decouper_fichier("Carte-LOCR-JP-OP-ScR-1E.png", "LOCR").unwrap();
        assert_eq!(
            infos.rarete_abbr, "ScR",
            "OP est un contexte, pas une rareté"
        );
    }

    #[test]
    fn sans_langue_reconnue_le_premier_jeton_devient_la_rarete() {
        let (_, infos) = decouper_fichier("Carte-RA05-UR-1E.png", "RA05").unwrap();
        assert_eq!(infos.langue, "");
        assert_eq!(infos.rarete_abbr, "UR");
    }

    #[test]
    fn seule_la_premiere_rarete_est_retenue() {
        let (_, infos) = decouper_fichier("Carte-RA05-EN-UR-ScR-1E.png", "RA05").unwrap();
        assert_eq!(infos.rarete_abbr, "UR");
    }

    #[test]
    fn les_jetons_minuscules_sont_normalises_sauf_la_rarete() {
        // Vérifié contre le Python : langue, édition et variante remontent en
        // majuscules, la rareté garde sa casse d'origine. C'est cette dernière
        // asymétrie qui fait vivre `PScR` et `StR`.
        let (segment, infos) = decouper_fichier("Carte-RA05-en-ur-1e-aa.png", "RA05").unwrap();
        assert_eq!(segment, "Carte");
        assert_eq!(infos.langue, "EN");
        assert_eq!(infos.rarete_abbr, "ur", "la rareté n'est jamais normalisée");
        assert_eq!(infos.edition, "1E");
        assert_eq!(infos.variante, "AA");

        // Le préfixe de set lui-même se retrouve sans égard à la casse.
        let (segment, _) = decouper_fichier("carte-ra05-EN-UR-1E.png", "RA05").unwrap();
        assert_eq!(segment, "carte");

        // Un fichier réduit au préfixe et à la langue : ni rareté ni édition.
        let (_, infos) = decouper_fichier("Carte-RA05-EN.png", "RA05").unwrap();
        assert_eq!(
            (infos.rarete_abbr.as_str(), infos.edition.as_str()),
            ("", "")
        );
        // Plus aucun jeton après le marqueur : rien à découper.
        assert!(decouper_fichier("Carte-RA05-.png", "RA05").is_none());
    }

    #[test]
    fn les_suffixes_numerotes_sont_reconnus() {
        let (_, infos) = decouper_fichier("Carte-RA05-EN-UR-1E-AA2.png", "RA05").unwrap();
        assert_eq!(infos.variante, "AA2");
        let (_, infos) = decouper_fichier("Carte-RA05-EN-UR-1E-ALT.png", "RA05").unwrap();
        assert_eq!(infos.variante, "ALT");
    }

    #[test]
    fn analyser_valide_ou_extrait_le_nom_de_carte() {
        let f = "VanquishSoulRazen-RA05-EN-UR-1E.png";
        assert!(analyser_fichier(f, "Vanquish Soul Razen", "RA05").is_some());
        assert!(analyser_fichier(f, "Dark Magician", "RA05").is_none());
        let (segment, cle, _) = analyser_fichier_set(f, "RA05").unwrap();
        assert_eq!(segment, "VanquishSoulRazen");
        assert_eq!(cle, "vanquishsoulrazen");
    }

    #[test]
    fn badge_et_libelle_depuis_l_uuid() {
        assert_eq!(
            badge_depuis_uuid("yugipedia:VanquishSoulRazen-RA05-EN-UR-1E-AA.png"),
            "AA"
        );
        assert_eq!(
            badge_depuis_uuid("yugipedia:RedEyesDarkDragoon-RA05-EN-UR-1E-EA.png"),
            "EA"
        );
        assert_eq!(
            badge_depuis_uuid("yugipedia:Carte-RA05-EN-UR-1E.png"),
            "Print"
        );
        // Pas un artwork externe.
        assert_eq!(badge_depuis_uuid("abcd-1234"), "");
        assert_eq!(libelle_depuis_uuid("abcd-1234"), "");
        assert_eq!(
            libelle_depuis_uuid("yugipedia:Carte-RA05-EN-UR-1E-AA.png"),
            "Alternate Artwork — Yugipedia (Carte-RA05-EN-UR-1E-AA.png)"
        );
        assert_eq!(
            libelle_depuis_uuid("yugipedia:Carte-RA05-EN-UR-1E.png"),
            "Print Yugipedia — Yugipedia (Carte-RA05-EN-UR-1E.png)"
        );
    }

    #[test]
    fn l_abreviation_passe_par_le_referentiel() {
        assert_eq!(abbr_rarete("Ultra Rare"), "UR");
        assert_eq!(abbr_rarete("ultra rare"), "UR");
        assert_eq!(abbr_rarete("Secret Rare"), "ScR");
        assert_eq!(abbr_rarete(""), "");
        assert_eq!(abbr_rarete("Rareté inconnue"), "");
        // Entrées mortes : le référentiel normalise avant la table.
        assert_eq!(abbr_rarete("Short Print"), "C", "et non SP");
        assert_eq!(abbr_rarete("Common Parallel Rare"), "NPR");
    }

    #[test]
    fn le_score_classe_langue_puis_rarete_puis_edition() {
        let infos = |lg: &str, rar: &str, ed: &str, var: &str| InfosFichier {
            fichier: String::new(),
            langue: lg.to_owned(),
            rarete_abbr: rar.to_owned(),
            edition: ed.to_owned(),
            variante: var.to_owned(),
        };
        assert_eq!(score(&infos("EN", "UR", "1E", ""), "EN", "UR"), 60 + 40 + 5);
        assert_eq!(score(&infos("EN", "UR", "1E", ""), "FR", "UR"), 30 + 40 + 5);
        assert_eq!(score(&infos("JP", "UR", "UE", ""), "FR", "UR"), 10 + 40);
        assert_eq!(score(&infos("", "", "", ""), "EN", ""), 0);
        // AA passe devant EA à égalité par ailleurs.
        assert!(
            score(&infos("EN", "UR", "1E", "AA"), "EN", "UR")
                > score(&infos("EN", "UR", "1E", "EA"), "EN", "UR")
        );
    }

    #[test]
    fn le_pool_de_variantes_lit_les_annotations() {
        let w = "== Main pool ==\n\
                 RA05-EN001; Carte; UR\n\
                 == Variant art pool ==\n\
                 RA05-EN083; Dark Magician; UR\n\
                 RA05-EN134; Vanquish Soul Razen; UR // description::(stamp artwork)\n\
                 RA05-EN141; Red-Eyes; UR // description::(extended art)\n";
        let pool = parser_variant_pool(w, "RA05");
        assert_eq!(pool.get("RA05-EN001"), None, "hors section variante");
        assert_eq!(pool.get("RA05-EN083").map(String::as_str), Some("AA"));
        assert_eq!(pool.get("RA05-EN134").map(String::as_str), Some("AA"));
        assert_eq!(pool.get("RA05-EN141").map(String::as_str), Some("EA"));
    }

    #[test]
    fn une_annotation_inconnue_qui_parle_d_illustration_vaut_aa() {
        let w = "RA05-EN200; Carte; UR // description::(3rd artwork)\n";
        assert_eq!(
            parser_variant_pool(w, "RA05")
                .get("RA05-EN200")
                .map(String::as_str),
            Some("AA")
        );
        // Une annotation sans « art » ne dit rien.
        let w = "RA05-EN201; Carte; UR // description::(alternate password)\n";
        assert!(parser_variant_pool(w, "RA05").is_empty());
    }

    /// Les formes relevées sur Yugipedia le 2026-09-30, pour les sets de
    /// l'utilisateur.
    #[test]
    fn les_illustrations_alternatives_se_lisent_sous_toutes_leurs_formes() {
        let w = "{{Set list|region=EN|\n\
                 RA02-EN006; Droll & Lock Bird; UR // description::(alternate art)\n\
                 RA02-EN047; Polymerization; UR\n\
                 RA04-EN106; Dark Magician; PlScR // description::(Arkana)\n\
                 RA04-EN106; Dark Magician; PlScR // description::(6th artwork)\n\
                 RA04-EN003; RE Darkness Metal; QCScR // description::(alternate artwork)\n\
                 RA04-EN001; Dark Magician; QCScR // description::(new artwork)\n\
                 RA05-EN141; Red-Eyes; UR // description::(extended art)\n";
        let alt: Vec<String> = ["RA02", "RA04", "RA05"]
            .iter()
            .flat_map(|p| numeros_illustration_alternative(w, p))
            .collect();
        assert_eq!(
            alt,
            ["RA02-EN006", "RA04-EN001", "RA04-EN003", "RA04-EN106"],
            "(extended art) n'est pas une autre illustration, Polymerization n'a rien"
        );
    }

    /// Une ligne `(extended art)` après la ligne normale n'efface pas ce que
    /// la colonne d'impression a dit.
    #[test]
    fn plusieurs_lignes_pour_un_numero_se_cumulent() {
        let w = "LOCH-JP027; I:P Masquerena; UR; New artwork\n\
                 LOCH-JP027; I:P Masquerena; GMR; New // description::(extended art)\n";
        assert!(numeros_illustration_alternative(w, "LOCH").contains("LOCH-JP027"));
        // Le pool historique, lui, garde la dernière famille vue — inchangé.
        assert_eq!(
            parser_variant_pool(w, "LOCH")
                .get("LOCH-JP027")
                .map(String::as_str),
            Some("EA")
        );
    }

    #[test]
    fn le_pool_est_vide_sans_signal() {
        assert!(parser_variant_pool("", "RA05").is_empty());
        assert!(parser_variant_pool("RA05-EN001; Carte; UR\n", "").is_empty());
        assert!(parser_variant_pool("du texte\n", "RA05").is_empty());
    }

    #[test]
    fn les_titres_candidats_suivent_l_ordre_d_essai() {
        assert_eq!(
            titres_set_list("Rarity Collection 5", "FR"),
            [
                "Set Card Lists:Rarity Collection 5 (TCG-FR)",
                "Set Card Lists:Rarity Collection 5 (TCG-EN)",
                "Set Card Lists:Rarity Collection 5 (OCG-FR)",
            ]
        );
        // En anglais, pas de repli à ajouter : deux titres seulement.
        assert_eq!(titres_set_list("RA05", "EN").len(), 2);
        assert_eq!(titres_set_list("RA05", "").len(), 2, "défaut = EN");
        assert!(titres_set_list("", "EN").is_empty());
    }

    #[test]
    fn l_extension_se_retire_comme_en_python() {
        assert_eq!(sans_extension("a.png"), "a");
        assert_eq!(sans_extension("a.b.png"), "a.b");
        assert_eq!(sans_extension("a"), "a");
        assert_eq!(
            sans_extension(".png"),
            ".png",
            "un point de tête n'est pas une extension"
        );
    }

    // ── Indexation d'un set ─────────────────────────────────────────────────

    fn distant(nom: &str) -> FichierDistant {
        FichierDistant {
            nom: nom.to_owned(),
            url: format!("https://ms.yugipedia.com/{nom}"),
        }
    }

    #[test]
    fn l_indexation_garde_les_fichiers_nus_comme_les_suffixes() {
        // Le suffixe ne dit PAS si le tirage est une variante : c'est la Set
        // list qui le dit. Écarter les nus ici ferait manquer RA05-EN083.
        let index = indexer_fichiers_set(
            &[
                distant("DarkMagician-LOCR-JP-UR.png"),
                distant("DarkMagician-LOCR-JP-UR-EA.png"),
            ],
            "LOCR",
            "JP",
            &[],
        );
        let candidats = index
            .get(&("darkmagician".to_owned(), "UR".to_owned()))
            .expect("indexé sous (carte, rareté)");
        assert_eq!(candidats.len(), 2);
        assert_eq!(candidats[0].infos.variante, "");
        assert_eq!(candidats[1].infos.variante, "EA");
    }

    #[test]
    fn l_indexation_ecarte_une_autre_langue_mais_garde_l_absence_de_langue() {
        let index = indexer_fichiers_set(
            &[
                distant("DarkMagician-LOCR-JP-UR.png"),
                distant("DarkMagician-LOCR-EN-UR.png"),
                distant("DarkMagician-LOCR-UR.png"),
            ],
            "LOCR",
            "JP",
            &[],
        );
        let tous: Vec<&str> = index
            .values()
            .flatten()
            .map(|c| c.infos.fichier.as_str())
            .collect();
        assert_eq!(tous.len(), 2, "l'anglais est écarté");
        assert!(
            tous.contains(&"DarkMagician-LOCR-UR.png"),
            "un fichier sans langue passe : rien ne dit qu'il est étranger"
        );
    }

    #[test]
    fn l_indexation_ecarte_les_cartes_hors_classeur_sauf_si_la_liste_est_vide() {
        let fichiers = [
            distant("DarkMagician-LOCR-JP-UR.png"),
            distant("BlueEyesWhiteDragon-LOCR-JP-UR.png"),
        ];
        let filtre = indexer_fichiers_set(&fichiers, "LOCR", "JP", &["Dark Magician".to_owned()]);
        assert_eq!(filtre.values().flatten().count(), 1);

        let sans_filtre = indexer_fichiers_set(&fichiers, "LOCR", "JP", &[]);
        assert_eq!(
            sans_filtre.values().flatten().count(),
            2,
            "une liste vide ne filtre rien"
        );
    }

    #[test]
    fn l_indexation_ecarte_ce_qui_n_a_pas_de_nom_de_carte_devant_le_prefixe() {
        // Covers et tapis de jeu : la recherche plein texte les remonte.
        let index = indexer_fichiers_set(
            &[
                distant("LOCR-BoosterJP.png"),
                distant("LOCR-JP-Playmat.png"),
            ],
            "LOCR",
            "JP",
            &[],
        );
        assert!(index.is_empty());
    }

    #[test]
    fn la_rarete_indexee_est_en_majuscules_mais_pas_celle_du_candidat() {
        // La clé d'index est normalisée ; la casse d'origine reste lisible sur
        // le candidat, parce que c'est elle qui s'affiche.
        let index = indexer_fichiers_set(
            &[distant("DarkMagician-LOCR-JP-PScR.png")],
            "LOCR",
            "JP",
            &[],
        );
        let ((_, rarete), candidats) = index.iter().next().expect("un candidat");
        assert_eq!(rarete, "PSCR");
        assert_eq!(candidats[0].infos.rarete_abbr, "PScR");
    }

    // ── Mise en forme ───────────────────────────────────────────────────────

    fn candidat_de(nom: &str) -> Candidat {
        indexer_fichiers_set(&[distant(nom)], "LOCR", "JP", &[])
            .into_values()
            .flatten()
            .next()
            .expect("un candidat")
    }

    #[test]
    fn l_artwork_pose_la_meme_url_en_vignette() {
        // Ce chemin ne produit jamais d'`art_url` : la vignette EST l'image.
        let art = formater_artwork(&candidat_de("DarkMagician-LOCR-JP-UR-EA.png"), "LOCR");
        assert_eq!(art.card_image_small, art.card_image_url);
        assert_eq!(
            art.card_image_uuid,
            "yugipedia:DarkMagician-LOCR-JP-UR-EA.png"
        );
        assert!(art.card_image_id < 0, "identifiant synthétique négatif");
    }

    #[test]
    fn les_etiquettes_disent_la_famille_et_le_detail() {
        let art = formater_artwork(&candidat_de("DarkMagician-LOCR-JP-UR-EA.png"), "LOCR");
        assert_eq!(art.badge, "EA");
        assert_eq!(art.libelle, "Extended Art (LOCR · JP · UR)");

        let nu = formater_artwork(&candidat_de("DarkMagician-LOCR-JP-UR-1E.png"), "LOCR");
        assert_eq!(nu.badge, "Print", "sans suffixe, un badge quand même");
        assert_eq!(nu.libelle, "Print Yugipedia (LOCR · JP · UR · 1E)");
    }

    #[test]
    fn une_variante_numerotee_garde_sa_famille() {
        // `AA2` doit donner « Alternate Artwork », pas le libellé par défaut.
        let art = formater_artwork(&candidat_de("DarkMagician-LOCR-JP-UR-AA2.png"), "LOCR");
        assert_eq!(art.badge, "AA2");
        assert!(
            art.libelle.starts_with("Alternate Artwork"),
            "obtenu : {}",
            art.libelle
        );
    }
}
