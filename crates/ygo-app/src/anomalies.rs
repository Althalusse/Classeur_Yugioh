// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Les artworks qu'un classeur devrait avoir et n'a pas.
//!
//! Portage d'`anomalie/anomalie_service.py` (686 l.).
//!
//! # Ce qu'est une anomalie, exactement
//!
//! Une carte peut exister en plusieurs illustrations — `card_image_uuid`
//! distincts pour un même `card_uuid`. Une **anomalie** est le constat que,
//! pour un tirage donné `(set_code, rareté)`, une source connaît l'artwork A
//! et pas l'artwork B, alors que B existe ailleurs pour la même carte.
//!
//! Ce n'est pas une erreur de notre part : c'est une lacune des données
//! amont, et elle se voit — la carte manque dans le classeur, ou n'y figure
//! que dans une de ses versions.
//!
//! # La comparaison va dans les deux sens, et ce n'est pas gratuit
//!
//! Le Python d'origine ne comparait que du premier artwork (par ordre
//! alphabétique d'uuid) vers les suivants. *Droll & Lock Bird* sur
//! `RA02-EN006` en donne le contre-exemple : son Art 2 porte l'uuid `486a…`
//! et son Art 1 l'uuid `56b5…`. Art 2 était donc le « A », Art 1 le « B », et
//! la direction qui manquait — `RA02` a l'Art 1, pas l'Art 2 — n'était jamais
//! calculée. La comparaison est bidirectionnelle, et le commentaire du Python
//! dit qu'elle l'est devenue pour cette carte-là.
//!
//! Le coût est en O(N²) sur le nombre d'artworks d'une carte. Mesuré sur la
//! base réelle : 81 441 tirages lus, la carte la plus fournie en porte une
//! quinzaine, et le scan complet tient en quelques secondes.
//!
//! # Ce que la détection ne décide pas
//!
//! Elle ne crée aucune ligne. Elle **constate**, et range le constat dans la
//! table `anomalies` de `cardinfo.db`. C'est [`corriger`] qui écrit dans le
//! classeur, une anomalie à la fois, et l'utilisateur qui choisit lesquelles.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use rusqlite::Connection;
use ygo_core::config::SourceImage;
use ygo_core::config::{a_suffixe_ocg, Config};
use ygo_core::image_source::url_image;
use ygo_core::paths::Paths;
use ygo_images::plan::{nom_de_fichier, Cible};

use crate::error::{AppError, Result};

/// Le DDL de la table, pour les bases qui ne l'ont pas encore.
///
/// Elle n'appartient pas au schéma d'initialisation — l'init n'écrit que huit
/// tables, et celle-ci survit aux reconstructions. C'est donc à ce module de
/// s'assurer qu'elle existe, exactement comme le faisait
/// `_ensure_anomalies_table`.
const DDL_ANOMALIES: &str = "
CREATE TABLE IF NOT EXISTS anomalies (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    name                TEXT    NOT NULL,
    art_a_image_uuid    TEXT    NOT NULL,
    art_b_image_uuid    TEXT    NOT NULL,
    art_index           INTEGER NOT NULL,
    set_code_prefix     TEXT    NOT NULL,
    missing_set_code    TEXT    NOT NULL,
    missing_set_rarity  TEXT    NOT NULL,
    image_url           TEXT,
    image_url_small     TEXT,
    image_id            INTEGER,
    corrige             INTEGER DEFAULT 0,
    UNIQUE(art_b_image_uuid, missing_set_code, missing_set_rarity)
)";

/// Les tables de `cardinfo.db` sans lesquelles le scan n'a rien à lire.
pub const TABLES_REQUISES: [&str; 4] = ["set_prints", "set_locales", "card_texts", "card_images"];

/// Une ligne du scan : un tirage, et l'artwork qui le porte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LigneScan {
    /// Nom anglais de la carte — c'est la clé de regroupement.
    pub nom: String,
    /// L'artwork.
    pub image_uuid: String,
    /// Le tirage.
    pub set_code: String,
    /// Sa rareté.
    pub rarity: String,
    /// Identifiant YGOPRODeck de l'illustration.
    pub image_id: Option<i64>,
    /// URL pleine résolution.
    pub image_url: String,
    /// URL de la vignette.
    pub image_url_small: String,
}

/// Un artwork manquant, tel que la table le stocke.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anomalie {
    /// `rowid` dans la table `anomalies`, ou `None` pour une anomalie
    /// **synthétique** — cf. [`artworks_alternatifs`].
    pub id: Option<i64>,
    /// Nom anglais de la carte.
    pub nom: String,
    /// L'artwork présent, celui qui atteste que le tirage existe.
    pub art_a_image_uuid: String,
    /// L'artwork absent, celui que la correction ajoutera.
    pub art_b_image_uuid: String,
    /// Le rang de l'artwork absent dans l'ordre des uuid, à partir de 1.
    /// Sert à l'affichage (« Art 2 »), jamais à l'appariement.
    pub art_index: u32,
    /// Le classeur concerné.
    pub set_code_prefix: String,
    /// Le tirage où l'artwork manque.
    pub missing_set_code: String,
    /// Sa rareté.
    pub missing_set_rarity: String,
    /// L'illustration à poser.
    pub image_url: String,
    /// Sa vignette.
    pub image_url_small: String,
    /// L'identifiant YGOPRODeck de l'illustration.
    pub image_id: Option<i64>,
    /// La correction a-t-elle déjà été appliquée ?
    pub corrige: bool,
}

/// Le classeur qu'un `set_code` désigne.
///
/// Un code TCG rend son préfixe nu ; un code OCG **garde son suffixe de
/// langue**, parce que c'est ainsi que son dossier s'appelle sur le disque.
///
/// ```
/// use ygo_app::anomalies::prefixe_classeur;
/// assert_eq!(prefixe_classeur("CROS-EN001"), "CROS");
/// assert_eq!(prefixe_classeur("RA02-EU006"), "RA02");
/// assert_eq!(prefixe_classeur("LOB-EN1"), "LOB");
/// assert_eq!(prefixe_classeur("LOCH-JP001"), "LOCH-JP");
/// assert_eq!(prefixe_classeur("CROS-JP002"), "CROS-JP");
/// // Sets multi-decks : `ENM` n'est pas un suffixe OCG.
/// assert_eq!(prefixe_classeur("L26D-ENM01"), "L26D");
/// assert_eq!(prefixe_classeur(""), "");
/// ```
#[must_use]
pub fn prefixe_classeur(set_code: &str) -> String {
    let s = set_code.trim().to_ascii_uppercase();
    let Some((tete, reste)) = s.split_once('-') else {
        return s;
    };
    let langue: String = reste
        .chars()
        .take_while(char::is_ascii_alphabetic)
        .collect();
    // `a_suffixe_ocg` porte déjà la liste des suffixes OCG et la même règle
    // de découpe : la réemployer évite deux listes à tenir d'accord.
    if a_suffixe_ocg(&format!("{tete}-{langue}")) {
        format!("{tete}-{langue}")
    } else {
        tete.to_owned()
    }
}

/// La clé de tri d'une anomalie — celle de l'écran classeur.
///
/// `(classeur, groupe de lettres, numéro, rareté, nom)`. Le groupe de lettres
/// sépare les sous-decks des sets multi-decks (`L26D-ENM01` avant
/// `L26D-ENS01`) ; il est vide partout ailleurs, et le tri se fait alors sur
/// le numéro seul.
///
/// Elle doit rester **alignée** sur `ygo_core::tri` : une anomalie qui
/// s'afficherait dans un autre ordre que la carte qu'elle concerne serait
/// introuvable.
#[must_use]
pub fn cle_de_tri(a: &Anomalie) -> (String, String, u32, String, String) {
    let suffixe = a
        .missing_set_code
        .rsplit_once('-')
        .map_or(a.missing_set_code.as_str(), |(_, s)| s);
    // Les deux premières lettres sont le code-langue Konami.
    let apres_langue = if suffixe.len() >= 2 && suffixe[..2].chars().all(|c| c.is_ascii_uppercase())
    {
        &suffixe[2..]
    } else {
        suffixe
    };
    let lettres: String = apres_langue
        .chars()
        .take_while(char::is_ascii_uppercase)
        .collect();
    let chiffres = &apres_langue[lettres.len()..];
    let numero = if !chiffres.is_empty() && chiffres.chars().all(|c| c.is_ascii_digit()) {
        chiffres.parse().unwrap_or(0)
    } else {
        // Ni lettres ni chiffres exploitables : le Python rend (0, "").
        return (
            a.set_code_prefix.clone(),
            String::new(),
            0,
            a.missing_set_rarity.clone(),
            a.nom.clone(),
        );
    };
    (
        a.set_code_prefix.clone(),
        lettres,
        numero,
        a.missing_set_rarity.clone(),
        a.nom.clone(),
    )
}

/// Trie des anomalies dans l'ordre de l'écran classeur.
pub fn trier(anomalies: &mut [Anomalie]) {
    anomalies.sort_by_key(cle_de_tri);
}

// ─────────────────────────────────────────────────────────────────────────────
// Regroupement pour l'écran — une décision par numéro, pas par rareté
// ─────────────────────────────────────────────────────────────────────────────

/// Un artwork proposé pour un numéro, avec la proposition de chaque rareté.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtworkPropose {
    /// Le rang affiché — « Art 2 ».
    pub art_index: u32,
    /// L'illustration proposée ; la même pour toutes les raretés du groupe.
    pub image_uuid: String,
    /// Une anomalie par rareté pour laquelle cet artwork est proposé, dans
    /// l'ordre des raretés du [`Numero`].
    pub par_rarete: Vec<Anomalie>,
}

impl ArtworkPropose {
    /// L'anomalie de cette rareté, si l'artwork est proposé pour elle.
    #[must_use]
    pub fn pour(&self, rarete: &str) -> Option<&Anomalie> {
        self.par_rarete
            .iter()
            .find(|a| a.missing_set_rarity == rarete)
    }

    /// Une anomalie représentative — pour l'image, qui ne dépend pas de la
    /// rareté.
    #[must_use]
    pub fn modele(&self) -> Option<&Anomalie> {
        self.par_rarete.first()
    }
}

/// Un numéro de classeur et tout ce que le scan propose pour lui.
///
/// # Pourquoi regrouper — 2026-09-21
///
/// L'écran montrait une ligne par anomalie, c'est-à-dire par **rareté** : la
/// même paire d'images revenait trois ou quatre fois. Sur l'installation
/// réelle, **379 lignes pour 205 décisions** ; `LOCR-JP`, 15 lignes pour 4.
/// Or l'image proposée ne dépend jamais de la rareté (vérifié : 0 groupe sur
/// 205 où elle diffère) — seule l'image **en place** peut en dépendre.
///
/// Le regroupement se fait par numéro, puis par artwork : 21 numéros de
/// l'installation ont plusieurs artworks proposés (jusqu'à 8 pour Dark
/// Magician), et chacun se choisit indépendamment, rareté par rareté.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Numero {
    /// Le classeur.
    pub classeur: String,
    /// Le numéro — `LOCR-JP038`.
    pub set_code: String,
    /// Le nom anglais.
    pub nom: String,
    /// Toutes les raretés concernées, dans l'ordre où elles arrivent.
    pub raretes: Vec<String>,
    /// Les artworks proposés, par rang.
    pub artworks: Vec<ArtworkPropose>,
}

impl Numero {
    /// Toutes les anomalies du numéro, tous artworks confondus.
    pub fn anomalies(&self) -> impl Iterator<Item = &Anomalie> {
        self.artworks.iter().flat_map(|a| a.par_rarete.iter())
    }

    /// Combien de propositions restent à poser.
    #[must_use]
    pub fn restantes(&self) -> usize {
        self.anomalies().filter(|a| !a.corrige).count()
    }
}

/// Regroupe des anomalies par numéro, puis par artwork.
///
/// **Fonction pure.** L'ordre d'entrée est celui de l'écran classeur
/// ([`trier`]) et il est conservé : un numéro prend la place de sa première
/// anomalie, une rareté celle de sa première apparition. Les artworks sont
/// rangés par rang, puis par uuid.
#[must_use]
pub fn regrouper(anomalies: Vec<Anomalie>) -> Vec<Numero> {
    let mut numeros: Vec<Numero> = Vec::new();
    let mut indice: HashMap<(String, String), usize> = HashMap::new();
    for a in anomalies {
        let cle = (a.set_code_prefix.clone(), a.missing_set_code.clone());
        let i = *indice.entry(cle).or_insert_with(|| {
            numeros.push(Numero {
                classeur: a.set_code_prefix.clone(),
                set_code: a.missing_set_code.clone(),
                nom: a.nom.clone(),
                raretes: Vec::new(),
                artworks: Vec::new(),
            });
            numeros.len() - 1
        });
        let Some(numero) = numeros.get_mut(i) else {
            continue;
        };
        if !numero.raretes.contains(&a.missing_set_rarity) {
            numero.raretes.push(a.missing_set_rarity.clone());
        }
        match numero
            .artworks
            .iter_mut()
            .find(|p| p.image_uuid == a.art_b_image_uuid)
        {
            Some(p) => p.par_rarete.push(a),
            None => numero.artworks.push(ArtworkPropose {
                art_index: a.art_index,
                image_uuid: a.art_b_image_uuid.clone(),
                par_rarete: vec![a],
            }),
        }
    }
    for n in &mut numeros {
        n.artworks
            .sort_by(|x, y| (x.art_index, &x.image_uuid).cmp(&(y.art_index, &y.image_uuid)));
        let ordre = n.raretes.clone();
        for p in &mut n.artworks {
            p.par_rarete.sort_by_key(|a| {
                ordre
                    .iter()
                    .position(|r| *r == a.missing_set_rarity)
                    .unwrap_or(usize::MAX)
            });
        }
    }
    numeros
}

/// Deux fichiers d'image ont-ils le **même contenu** ?
///
/// Rend `None` si l'un des deux manque : on ne sait pas, et l'écran ne doit
/// pas affirmer.
///
/// # Pourquoi comparer les octets, et pas les identifiants — 2026-09-21
///
/// La refonte de l'écran a montré les images en grand, et révélé que sur
/// `LOCR-JP` les quatre « artworks alternatifs » étaient **la même image**
/// que celle en place, octet pour octet. YGOPRODeck garde deux identifiants
/// pour une carte sortie en OCG avant le TCG : un temporaire, au-delà de
/// 100 000 000, et le vrai code une fois connu — même illustration. YGOJSON
/// les liste comme deux images, et le scan comme deux artworks. Sur les 33
/// propositions comparables de l'installation réelle, **8** étaient ce
/// doublon (quatre cartes, sur `LOCR-JP` et `MP25`).
///
/// Une règle sur les identifiants (« au-delà de 100 000 000 ») serait une
/// supposition sur la façon dont YGOPRODeck numérote. Le contenu, lui, ne
/// ment pas.
#[must_use]
pub fn images_identiques(dossier: &std::path::Path, a: &str, b: &str) -> Option<bool> {
    if a == b {
        return Some(true);
    }
    let x = std::fs::read(dossier.join(a)).ok()?;
    let y = std::fs::read(dossier.join(b)).ok()?;
    Some(x == y)
}

/// Les tirages d'un artwork : `(set_code, rareté)`, sans doublon et ordonnés.
type Tirages<'a> = BTreeSet<(&'a str, &'a str)>;

/// Les artworks d'une carte, chacun avec ses tirages, dans l'ordre des uuid —
/// c'est cet ordre qui décide de l'`art_index`.
type Artworks<'a> = BTreeMap<&'a str, Tirages<'a>>;

/// Toutes les cartes du scan, par nom anglais.
type ParNom<'a> = BTreeMap<&'a str, Artworks<'a>>;

/// Détecte les artworks manquants — **fonction pure**.
///
/// `classeurs` borne le résultat à ce que l'installation possède : une
/// anomalie sur un set qu'on ne collectionne pas n'intéresse personne, et
/// elle serait de toute façon incorrigible.
///
/// Le résultat est trié, et **dédoublonné** sur `(art_b, code, rareté)` — la
/// clé unique de la table. Sans quoi trois artworks A qui attestent tous le
/// même tirage produiraient trois fois la même anomalie, et l'`INSERT OR
/// IGNORE` masquerait l'excès sans qu'on sache combien.
#[must_use]
pub fn detecter(lignes: &[LigneScan], classeurs: &HashSet<String>) -> Vec<Anomalie> {
    // `BTreeMap` partout : l'ordre des uuid décide de l'`art_index`, il ne
    // peut pas dépendre d'un hachage.
    let mut par_nom: ParNom<'_> = BTreeMap::new();
    let mut meta: HashMap<&str, &LigneScan> = HashMap::new();
    for l in lignes {
        par_nom
            .entry(l.nom.as_str())
            .or_default()
            .entry(l.image_uuid.as_str())
            .or_default()
            .insert((l.set_code.as_str(), l.rarity.as_str()));
        meta.entry(l.image_uuid.as_str()).or_insert(l);
    }

    let mut vues: HashSet<(&str, &str, &str)> = HashSet::new();
    let mut anomalies = Vec::new();
    for artworks in par_nom.values() {
        if artworks.len() < 2 {
            continue;
        }
        let uuids: Vec<&str> = artworks.keys().copied().collect();
        for (i, art_a) in uuids.iter().enumerate() {
            for (j, art_b) in uuids.iter().enumerate() {
                if i == j {
                    continue;
                }
                let (Some(prints_a), Some(prints_b)) = (artworks.get(art_a), artworks.get(art_b))
                else {
                    continue;
                };
                for (code, rarete) in prints_a.difference(prints_b) {
                    let prefixe = prefixe_classeur(code);
                    if !classeurs.contains(&prefixe) {
                        continue;
                    }
                    if !vues.insert((art_b, code, rarete)) {
                        continue;
                    }
                    let source = meta.get(art_b);
                    anomalies.push(Anomalie {
                        id: None,
                        nom: source.map(|s| s.nom.clone()).unwrap_or_default(),
                        art_a_image_uuid: (*art_a).to_owned(),
                        art_b_image_uuid: (*art_b).to_owned(),
                        art_index: u32::try_from(j + 1).unwrap_or(1),
                        set_code_prefix: prefixe,
                        missing_set_code: (*code).to_owned(),
                        missing_set_rarity: (*rarete).to_owned(),
                        image_url: source.map(|s| s.image_url.clone()).unwrap_or_default(),
                        image_url_small: source
                            .map(|s| s.image_url_small.clone())
                            .unwrap_or_default(),
                        image_id: source.and_then(|s| s.image_id),
                        corrige: false,
                    });
                }
            }
        }
    }
    trier(&mut anomalies);
    anomalies
}

// ─────────────────────────────────────────────────────────────────────────────
// Lecture de cardinfo.db
// ─────────────────────────────────────────────────────────────────────────────

/// Les tables requises qui manquent à `cardinfo.db`.
///
/// Vide si tout est là. Une base absente rend la liste entière — c'est le cas
/// du premier démarrage, avant l'initialisation.
///
/// # Errors
///
/// N'en rend jamais : une base illisible est une base incomplète.
pub fn tables_manquantes(paths: &Paths) -> Vec<String> {
    let toutes = || TABLES_REQUISES.iter().map(|t| (*t).to_owned()).collect();
    let Ok(conn) = ygo_db::connexion::ouvrir_lecture_seule(paths.cardinfo_db()) else {
        return toutes();
    };
    let mut manquantes = Vec::new();
    for table in TABLES_REQUISES {
        let existe: bool = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
                [table],
                |_| Ok(true),
            )
            .unwrap_or(false);
        if !existe {
            manquantes.push(table.to_owned());
        }
    }
    manquantes
}

/// Lit les tirages de `cardinfo.db`, dans les langues actives.
///
/// # Errors
///
/// Rend une erreur si la base est illisible ou incomplète.
pub fn lignes_de_scan(paths: &Paths) -> Result<Vec<LigneScan>> {
    let manquantes = tables_manquantes(paths);
    if !manquantes.is_empty() {
        return Err(AppError::Creation(format!(
            "cardinfo.db n'est pas initialisée — tables manquantes : {}. \
             Lancez « MAJ BDD » — Options ▸ Base de référence — avant de scanner.",
            manquantes.join(", ")
        )));
    }
    let conn = ygo_db::connexion::ouvrir_lecture_seule(paths.cardinfo_db())
        .map_err(|e| AppError::Creation(format!("cardinfo.db : {e}")))?;
    let langues = Config::charger(paths.app_config())
        .langues_locales_actives()
        .join("','");
    // Les langues viennent d'une constante du code, jamais de l'utilisateur :
    // les interpoler est sans risque, et évite un nombre variable de `?`.
    let sql = format!(
        "SELECT ct.name, ci.uuid, sp.set_code, sp.rarity,
                ci.ygoprodeck_image_id, ci.card_url, ci.art_url
           FROM set_prints sp
           JOIN set_locales sl ON sl.id = sp.set_locale_id AND sl.language IN ('{langues}')
           JOIN card_texts ct ON ct.card_uuid = sp.card_uuid AND ct.language = 'en'
           JOIN card_images ci ON ci.uuid = sp.card_image_uuid
          WHERE sp.set_code IS NOT NULL
          ORDER BY ct.name, ci.ygoprodeck_image_id"
    );
    let mut requete = conn.prepare(&sql)?;
    let lignes = requete
        .query_map([], |l| {
            Ok(LigneScan {
                nom: l.get::<_, Option<String>>(0)?.unwrap_or_default(),
                image_uuid: l.get::<_, Option<String>>(1)?.unwrap_or_default(),
                set_code: l.get::<_, Option<String>>(2)?.unwrap_or_default(),
                rarity: l.get::<_, Option<String>>(3)?.unwrap_or_default(),
                image_id: l.get(4)?,
                image_url: l.get::<_, Option<String>>(5)?.unwrap_or_default(),
                image_url_small: l.get::<_, Option<String>>(6)?.unwrap_or_default(),
            })
        })
        .and_then(Iterator::collect::<rusqlite::Result<Vec<_>>>)?;
    Ok(lignes)
}

// ─────────────────────────────────────────────────────────────────────────────
// La table `anomalies`
// ─────────────────────────────────────────────────────────────────────────────

/// Ouvre `cardinfo.db` en écriture, la table `anomalies` garantie présente.
fn ouvrir_table(paths: &Paths) -> Result<Connection> {
    let conn = ygo_db::connexion::ouvrir(paths.cardinfo_db())
        .map_err(|e| AppError::Creation(format!("cardinfo.db : {e}")))?;
    conn.execute_batch(DDL_ANOMALIES)?;
    Ok(conn)
}

/// Ce qu'un scan a trouvé.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Bilan {
    /// Anomalies détectées par cette passe.
    pub detectees: usize,
    /// Nouvelles lignes écrites — les autres existaient déjà.
    pub ajoutees: usize,
    /// Lignes retirées parce que leur classeur n'existe plus.
    pub retirees: usize,
    /// Total en base après la passe.
    pub total: usize,
}

/// Scanne `cardinfo.db` et met la table `anomalies` à jour.
///
/// # Ce que le scan préserve
///
/// L'écriture est un `INSERT OR IGNORE` : une anomalie déjà connue **garde
/// son drapeau `corrige`**. Rescanner ne défait donc pas le travail déjà
/// fait, et c'est ce qui rend la passe rejouable sans y penser.
///
/// # Errors
///
/// Rend une erreur si `cardinfo.db` est absente, incomplète, ou en lecture
/// seule.
pub fn scanner(paths: &Paths) -> Result<Bilan> {
    let classeurs: HashSet<String> = paths.classeurs_existants().into_iter().collect();
    if classeurs.is_empty() {
        return Ok(Bilan::default());
    }
    let lignes = lignes_de_scan(paths)?;
    let anomalies = detecter(&lignes, &classeurs);

    let mut conn = ouvrir_table(paths)?;
    let transaction = conn.transaction()?;
    let mut ajoutees = 0;
    {
        let mut insertion = transaction.prepare(
            "INSERT OR IGNORE INTO anomalies
                (name, art_a_image_uuid, art_b_image_uuid, art_index,
                 set_code_prefix, missing_set_code, missing_set_rarity,
                 image_url, image_url_small, image_id)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        )?;
        for a in &anomalies {
            ajoutees += insertion.execute(rusqlite::params![
                a.nom,
                a.art_a_image_uuid,
                a.art_b_image_uuid,
                a.art_index,
                a.set_code_prefix,
                a.missing_set_code,
                a.missing_set_rarity,
                a.image_url,
                a.image_url_small,
                a.image_id,
            ])?;
        }
    }
    // Les anomalies des classeurs disparus n'ont plus de sens : elles
    // désigneraient une base qui n'existe pas.
    let liste: Vec<&str> = classeurs.iter().map(String::as_str).collect();
    let trous = vec!["?"; liste.len()].join(",");
    let retirees = transaction.execute(
        &format!("DELETE FROM anomalies WHERE set_code_prefix NOT IN ({trous})"),
        rusqlite::params_from_iter(liste),
    )?;
    let total: i64 = transaction.query_row("SELECT COUNT(*) FROM anomalies", [], |l| l.get(0))?;
    transaction.commit()?;
    Ok(Bilan {
        detectees: anomalies.len(),
        ajoutees,
        retirees,
        total: usize::try_from(total).unwrap_or(0),
    })
}

/// Lit les anomalies stockées, triées comme l'écran classeur.
///
/// # Errors
///
/// Rend une erreur si `cardinfo.db` n'est pas accessible en écriture — la
/// table est créée si elle manque.
pub fn lire(paths: &Paths, classeur: Option<&str>) -> Result<Vec<Anomalie>> {
    let conn = ouvrir_table(paths)?;
    let colonnes = "id, name, art_a_image_uuid, art_b_image_uuid, art_index,
                    set_code_prefix, missing_set_code, missing_set_rarity,
                    image_url, image_url_small, image_id, corrige";
    let lire_ligne = |l: &rusqlite::Row<'_>| {
        Ok(Anomalie {
            id: l.get(0)?,
            nom: l.get::<_, Option<String>>(1)?.unwrap_or_default(),
            art_a_image_uuid: l.get::<_, Option<String>>(2)?.unwrap_or_default(),
            art_b_image_uuid: l.get::<_, Option<String>>(3)?.unwrap_or_default(),
            art_index: l.get::<_, Option<u32>>(4)?.unwrap_or(1),
            set_code_prefix: l.get::<_, Option<String>>(5)?.unwrap_or_default(),
            missing_set_code: l.get::<_, Option<String>>(6)?.unwrap_or_default(),
            missing_set_rarity: l.get::<_, Option<String>>(7)?.unwrap_or_default(),
            image_url: l.get::<_, Option<String>>(8)?.unwrap_or_default(),
            image_url_small: l.get::<_, Option<String>>(9)?.unwrap_or_default(),
            image_id: l.get(10)?,
            corrige: l.get::<_, Option<i64>>(11)?.unwrap_or(0) != 0,
        })
    };
    let mut anomalies = match classeur {
        Some(code) => {
            let mut requete = conn.prepare(&format!(
                "SELECT {colonnes} FROM anomalies WHERE set_code_prefix = ?1"
            ))?;
            requete
                .query_map([code], lire_ligne)
                .and_then(Iterator::collect::<rusqlite::Result<Vec<_>>>)?
        }
        None => {
            let mut requete = conn.prepare(&format!("SELECT {colonnes} FROM anomalies"))?;
            requete
                .query_map([], lire_ligne)
                .and_then(Iterator::collect::<rusqlite::Result<Vec<_>>>)?
        }
    };
    trier(&mut anomalies);
    Ok(anomalies)
}

/// Les classeurs qui portent au moins une anomalie.
///
/// # Errors
///
/// Voir [`lire`].
pub fn classeurs_avec_anomalies(paths: &Paths) -> Result<Vec<String>> {
    let conn = ouvrir_table(paths)?;
    let mut requete =
        conn.prepare("SELECT DISTINCT set_code_prefix FROM anomalies ORDER BY set_code_prefix")?;
    let codes = requete
        .query_map([], |l| l.get(0))
        .and_then(Iterator::collect::<rusqlite::Result<Vec<String>>>)?;
    Ok(codes)
}

/// Pose ou retire le drapeau `corrige` d'une anomalie stockée.
///
/// Une anomalie synthétique (`id` absent) n'est pas en base : l'appel est
/// alors sans effet, et ce n'est pas une erreur.
fn marquer(paths: &Paths, anomalie: &Anomalie, corrige: bool) -> Result<()> {
    let Some(id) = anomalie.id else {
        return Ok(());
    };
    let conn = ouvrir_table(paths)?;
    conn.execute(
        "UPDATE anomalies SET corrige = ?1 WHERE id = ?2",
        (i64::from(corrige), id),
    )?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// L'illustration
// ─────────────────────────────────────────────────────────────────────────────

/// Le fichier qu'une illustration porte dans `img/small`.
///
/// Exactement le même calcul que pour les cartes du classeur —
/// [`url_image`] puis [`nom_de_fichier`] —, et c'est ce qui compte : l'aperçu
/// d'une anomalie et la carte qu'elle deviendra une fois posée partagent le
/// **même fichier**. Aucun cache en double, et l'image déjà téléchargée pour
/// un autre classeur sert telle quelle.
#[must_use]
pub fn fichier_de(image_url: &str, image_id: Option<i64>, source: SourceImage) -> Option<String> {
    let url = url_image(source, Some(image_url), image_id)?;
    nom_de_fichier(&url)
}

/// L'illustration d'une anomalie, telle qu'on l'affichera.
#[must_use]
pub fn fichier_image(anomalie: &Anomalie, source: SourceImage) -> Option<String> {
    fichier_de(&anomalie.image_url, anomalie.image_id, source)
}

/// La cible de téléchargement de l'aperçu d'une anomalie.
///
/// Rend `None` si le fichier est déjà là : un aperçu ne se retélécharge pas.
#[must_use]
pub fn apercu_a_telecharger(
    paths: &Paths,
    anomalie: &Anomalie,
    source: SourceImage,
) -> Option<Cible> {
    let fichier = fichier_image(anomalie, source)?;
    let destination = paths.img_small().join(&fichier);
    if destination.is_file() {
        return None;
    }
    let url_primaire = url_image(source, Some(&anomalie.image_url), anomalie.image_id)?;
    // Le repli : l'autre source de la même carte. Sans identifiant il n'y en
    // a pas, et l'URL stockée est tout ce qu'on a.
    let url_repli = anomalie
        .image_id
        .filter(|id| *id > 0)
        .map(|id| match source {
            SourceImage::Ygoprodeck => anomalie.image_url.clone(),
            SourceImage::Yugipedia => ygo_core::image_source::url_ygoprodeck(id),
        })
        .filter(|u| !u.is_empty() && *u != url_primaire);
    Some(Cible {
        url_primaire,
        url_repli,
        destination,
    })
}

/// L'illustration que le classeur porte **déjà** pour chaque tirage.
///
/// `(set_code, rareté)` → nom de fichier. C'est le « avant » de la
/// comparaison : sans lui, l'écran montre un artwork sans dire lequel il
/// remplace ou complète.
#[must_use]
pub fn artworks_en_place(
    paths: &Paths,
    classeur: &str,
    source: SourceImage,
) -> HashMap<(String, String), String> {
    let Ok(conn) = ygo_db::connexion::ouvrir_lecture_seule(paths.classeur_db(classeur)) else {
        return HashMap::new();
    };
    let Ok(mut requete) = conn.prepare(
        "SELECT COALESCE(set_code,''), COALESCE(rarity,''),
                COALESCE(card_image_url,''), card_image_id
           FROM cards ORDER BY rowid",
    ) else {
        return HashMap::new();
    };
    let lignes: Vec<(String, String, String, Option<i64>)> = requete
        .query_map([], |l| Ok((l.get(0)?, l.get(1)?, l.get(2)?, l.get(3)?)))
        .and_then(Iterator::collect::<rusqlite::Result<Vec<_>>>)
        .unwrap_or_default();
    let mut en_place = HashMap::new();
    for (code, rarete, url, id) in lignes {
        if let Some(fichier) = fichier_de(&url, id, source) {
            // Le premier gagne : c'est la ligne d'origine, celle dont les
            // anomalies proposent une variante.
            en_place.entry((code, rarete)).or_insert(fichier);
        }
    }
    en_place
}

/// Toutes les propositions d'artwork pour **une** carte d'un classeur.
///
/// Réunit les deux origines, comme le fait le dialogue du Python :
///
/// - les anomalies du scan, qui savent qu'un autre set atteste cet artwork ;
/// - les artworks que `cardinfo.db` connaît pour la carte et que le classeur
///   n'a pas, que le scan ne voit pas parce qu'aucun autre set ne les porte.
///
/// Les secondes ne sont ajoutées que si leur illustration n'est pas déjà
/// proposée par les premières.
///
/// # Errors
///
/// Rend une erreur si la table `anomalies` est inaccessible.
pub fn propositions_pour_carte(
    paths: &Paths,
    classeur: &str,
    set_code: &str,
) -> Result<Vec<Anomalie>> {
    let mut propositions: Vec<Anomalie> = lire(paths, Some(classeur))?
        .into_iter()
        .filter(|a| a.missing_set_code == set_code)
        .collect();
    let deja: HashSet<i64> = propositions.iter().filter_map(|a| a.image_id).collect();
    propositions.extend(
        artworks_alternatifs(paths, classeur, set_code)
            .into_iter()
            .filter(|a| a.image_id.is_some_and(|id| !deja.contains(&id))),
    );
    trier(&mut propositions);
    Ok(propositions)
}

// ─────────────────────────────────────────────────────────────────────────────
// Correction
// ─────────────────────────────────────────────────────────────────────────────

/// Ce qu'une correction a fait.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Correction {
    /// La ligne a été créée dans le classeur.
    Ajoutee {
        /// Le classeur touché.
        classeur: String,
        /// Le `rowid` de la ligne créée.
        rowid: i64,
    },
    /// L'artwork était déjà là — l'anomalie est marquée corrigée.
    DejaPresente,
    /// Aucune ligne du même `(nom, tirage, rareté)` n'existe : il n'y a rien
    /// d'où hériter, et rien à corriger.
    SansTemoin,
    /// Le classeur n'est pas sur le disque.
    ClasseurAbsent,
}

impl Correction {
    /// A-t-elle écrit quelque chose ?
    #[must_use]
    pub fn a_ecrit(&self) -> bool {
        matches!(self, Self::Ajoutee { .. })
    }
}

/// Ajoute au classeur la ligne de l'artwork manquant.
///
/// # Ce que la nouvelle ligne hérite, et pourquoi
///
/// **Tout sauf l'illustration** : `card_uuid`, nom, nom français, nom de set,
/// statistiques, rareté, code de rareté, et surtout le `sort_order` de
/// l'Art A. C'est ce dernier qui place l'artwork alternatif **juste à côté**
/// de l'original dans le classeur, et non en fin de set.
///
/// # Le témoin : même rareté d'abord, même numéro à défaut
///
/// Le Python exigeait une ligne de `(nom, tirage, rareté)` exacte, sans quoi
/// il abandonnait. Sur les classeurs réels, deux propositions s'y heurtaient :
/// `RA05-EN072` et `RA05-EN080` en `Common`, que le classeur ne porte qu'en
/// Collector's, Platinum Secret, Secret, Starlight, Super et Ultimate. La
/// carte est là, sa rareté Common n'y est pas — et la source dit qu'elle
/// existe.
///
/// Le repli est celui que `artworks::inserer_ligne` applique déjà :
/// **même rareté d'abord, à défaut n'importe quelle ligne du même numéro**.
/// La rareté visée est alors **imposée**, et son `rarity_code` repris de la
/// table d'YGOPRODeck — celle que la colonne parle — plutôt qu'hérité du
/// témoin, qui désignerait une autre rareté. Le code Scanflip, lui, n'a rien
/// à faire là : il dit `SCR` quand la colonne attend `ScR`.
///
/// Elle naît non possédée. Ajouter un artwork ne dit rien de ce que
/// l'utilisateur a dans ses classeurs physiques.
///
/// # Errors
///
/// Rend une erreur si le classeur ne peut pas être ouvert ou écrit.
pub fn corriger(paths: &Paths, anomalie: &Anomalie) -> Result<Correction> {
    let chemin = paths.classeur_db(&anomalie.set_code_prefix);
    if !chemin.is_file() {
        return Ok(Correction::ClasseurAbsent);
    }
    let mut conn = ygo_db::connexion::ouvrir(chemin)
        .map_err(|e| AppError::Creation(format!("{} : {e}", anomalie.set_code_prefix)))?;
    let transaction = conn.transaction()?;

    // Le témoin : la ligne dont tout sera hérité. Même rareté d'abord — c'est
    // la source la plus fidèle —, n'importe quelle ligne du même numéro
    // ensuite. Les deux requêtes sont rendues optionnelles séparément : un
    // repli ne doit pas masquer une erreur SQL de la première.
    const COLONNES: &str = "card_uuid, card_image_uuid, set_code, rarity, rarity_code,
             set_name, name, name_fr, card_image_url, card_image_small,
             card_image_id, sort_order,
             card_type, atk, def_val, level, attribute, race";
    let ligne = |l: &rusqlite::Row<'_>| (0..18).map(|i| l.get(i)).collect();
    let mut meme_rarete = true;
    let mut source: Option<Vec<rusqlite::types::Value>> = transaction
        .prepare(&format!(
            "SELECT {COLONNES} FROM cards
              WHERE name = ?1 AND set_code = ?2 AND rarity = ?3
              ORDER BY rowid LIMIT 1"
        ))?
        .query_row(
            (
                &anomalie.nom,
                &anomalie.missing_set_code,
                &anomalie.missing_set_rarity,
            ),
            ligne,
        )
        .ok();
    if source.is_none() {
        meme_rarete = false;
        source = transaction
            .prepare(&format!(
                "SELECT {COLONNES} FROM cards
                  WHERE name = ?1 AND set_code = ?2
                  ORDER BY rowid LIMIT 1"
            ))?
            .query_row((&anomalie.nom, &anomalie.missing_set_code), ligne)
            .ok();
    }
    let Some(source) = source else {
        drop(transaction);
        marquer(paths, anomalie, true)?;
        return Ok(Correction::SansTemoin);
    };

    // Déjà là ? L'identifiant d'image d'abord, l'URL à défaut — c'est
    // l'ordre du Python, et le seul qui marche quand l'identifiant manque.
    let deja: Option<i64> = match (anomalie.image_id, anomalie.image_url.as_str()) {
        (Some(id), _) => transaction
            .query_row(
                "SELECT rowid FROM cards
                  WHERE name = ?1 AND set_code = ?2 AND rarity = ?3 AND card_image_id = ?4",
                rusqlite::params![
                    anomalie.nom,
                    anomalie.missing_set_code,
                    anomalie.missing_set_rarity,
                    id
                ],
                |l| l.get(0),
            )
            .ok(),
        (None, url) if !url.is_empty() => transaction
            .query_row(
                "SELECT rowid FROM cards
                  WHERE name = ?1 AND set_code = ?2 AND rarity = ?3 AND card_image_url = ?4",
                rusqlite::params![
                    anomalie.nom,
                    anomalie.missing_set_code,
                    anomalie.missing_set_rarity,
                    url
                ],
                |l| l.get(0),
            )
            .ok(),
        // Ni identifiant ni URL : il n'y a rien à poser.
        (None, _) => {
            drop(transaction);
            return Ok(Correction::SansTemoin);
        }
    };
    if deja.is_some() {
        drop(transaction);
        marquer(paths, anomalie, true)?;
        return Ok(Correction::DejaPresente);
    }

    let prendre = |i: usize| {
        source
            .get(i)
            .cloned()
            .unwrap_or(rusqlite::types::Value::Null)
    };
    let url_finale = if anomalie.image_url.is_empty() {
        prendre(8)
    } else {
        rusqlite::types::Value::Text(anomalie.image_url.clone())
    };
    let id_final = anomalie
        .image_id
        .map_or_else(|| prendre(10), rusqlite::types::Value::Integer);
    let uuid_final = if anomalie.art_b_image_uuid.is_empty() {
        prendre(1)
    } else {
        rusqlite::types::Value::Text(anomalie.art_b_image_uuid.clone())
    };
    // Un témoin d'une autre rareté ne dicte pas la rareté : c'est celle de
    // l'anomalie qui vaut, et son code se retrouve dans le **bon**
    // vocabulaire.
    //
    // La colonne porte le `set_rarity_code` d'YGOPRODeck — `ScR` pour Secret
    // Rare —, pas le code Scanflip, qui dit `SCR` pour la même rareté. Les
    // deux tables existent et sont extraites de leurs sources ; c'est la
    // première qu'il faut ici. `canon::abreviation_ygoprodeck` la renverse,
    // ce qui est sans ambiguïté sur les 25 entrées de cette source.
    //
    // `None` là où YGOPRODeck n'a pas d'abréviation : vide plutôt
    // qu'inventé, comme partout ailleurs.
    let (rarete_finale, code_rarete) = if meme_rarete {
        (prendre(3), prendre(4))
    } else {
        let code = ygo_core::rarity::canon::abreviation_ygoprodeck(&anomalie.missing_set_rarity)
            .map_or(rusqlite::types::Value::Null, |c| {
                rusqlite::types::Value::Text(c.to_owned())
            });
        (
            rusqlite::types::Value::Text(anomalie.missing_set_rarity.clone()),
            code,
        )
    };

    transaction.execute(
        "INSERT INTO cards
           (card_uuid, card_image_uuid, set_code, rarity, rarity_code,
            set_name, name, name_fr,
            card_image_url, card_image_small, card_image_id, sort_order,
            card_type, atk, def_val, level, attribute, race,
            possessed, quantite, qualite, is_custom)
         VALUES (?1,?2,?3,?4,?5, ?6,?7,?8, ?9,?10,?11, ?12, ?13,?14,?15,?16,?17,?18, 0,0,NULL,0)",
        rusqlite::params![
            prendre(0),
            uuid_final,
            prendre(2),
            rarete_finale,
            code_rarete,
            prendre(5),
            prendre(6),
            prendre(7),
            url_finale,
            prendre(9),
            id_final,
            prendre(11),
            prendre(12),
            prendre(13),
            prendre(14),
            prendre(15),
            prendre(16),
            prendre(17),
        ],
    )?;
    let rowid = transaction.last_insert_rowid();
    transaction.commit()?;
    marquer(paths, anomalie, true)?;
    Ok(Correction::Ajoutee {
        classeur: anomalie.set_code_prefix.clone(),
        rowid,
    })
}

/// Défait une correction : retire la ligne ajoutée.
///
/// # Errors
///
/// Rend une erreur si le classeur ne peut pas être écrit.
pub fn annuler(paths: &Paths, anomalie: &Anomalie) -> Result<bool> {
    let chemin = paths.classeur_db(&anomalie.set_code_prefix);
    if !chemin.is_file() {
        return Ok(false);
    }
    let conn = ygo_db::connexion::ouvrir(chemin)
        .map_err(|e| AppError::Creation(format!("{} : {e}", anomalie.set_code_prefix)))?;
    // La ligne n'est retirée que si elle n'a rien à perdre : une quantité
    // saisie depuis la correction est le signe que l'utilisateur possède
    // vraiment cet artwork. Le Python ne s'en protégeait pas.
    let efface = match (anomalie.image_id, anomalie.image_url.as_str()) {
        (Some(id), _) => conn.execute(
            "DELETE FROM cards
              WHERE name = ?1 AND set_code = ?2 AND rarity = ?3 AND card_image_id = ?4
                AND COALESCE(quantite, 0) = 0",
            rusqlite::params![
                anomalie.nom,
                anomalie.missing_set_code,
                anomalie.missing_set_rarity,
                id
            ],
        )?,
        (None, url) if !url.is_empty() => conn.execute(
            "DELETE FROM cards
              WHERE name = ?1 AND set_code = ?2 AND rarity = ?3 AND card_image_url = ?4
                AND COALESCE(quantite, 0) = 0",
            rusqlite::params![
                anomalie.nom,
                anomalie.missing_set_code,
                anomalie.missing_set_rarity,
                url
            ],
        )?,
        (None, _) => 0,
    };
    if efface > 0 {
        marquer(paths, anomalie, false)?;
    }
    Ok(efface > 0)
}

/// Applique toutes les anomalies connues d'un classeur qui vient de naître.
///
/// Portage d'`appliquer_overrides_sur_classeur_neuf` : une création efface le
/// travail de correction, puisqu'elle réécrit le classeur depuis les sources.
/// Rejouer les anomalies connues le rétablit.
///
/// # Errors
///
/// Rend une erreur si la table est illisible.
pub fn appliquer_sur_classeur_neuf(paths: &Paths, code: &str) -> Result<usize> {
    let anomalies = lire(paths, Some(code))?;
    let mut posees = 0;
    for a in &anomalies {
        if corriger(paths, a)?.a_ecrit() {
            posees += 1;
        }
    }
    Ok(posees)
}

/// Retrouve le `card_uuid` d'une carte à partir d'une de ses illustrations.
///
/// Le pont entre les deux vocabulaires d'identifiants : `card_image_id`, que
/// le classeur porte toujours, est l'`ygoprodeck_image_id` de `card_images`,
/// qui porte le `card_uuid`. Les identifiants **négatifs** sont écartés — ce
/// sont les artworks externes fabriqués par la passe Yugipedia
/// (cf. [`ygo_core::image_source::est_image_externe`]), qui ne correspondent à
/// aucune ligne de `card_images`.
///
/// Le premier identifiant qui donne une réponse gagne : toutes les lignes d'un
/// même `set_code` désignent la même carte, seule l'illustration change.
fn uuid_par_illustration(cardinfo: &Connection, illustrations: &HashSet<i64>) -> Option<String> {
    let mut candidats: Vec<i64> = illustrations.iter().copied().filter(|i| *i > 0).collect();
    candidats.sort_unstable();
    let mut requete = cardinfo
        .prepare("SELECT card_uuid FROM card_images WHERE ygoprodeck_image_id = ?1 LIMIT 1")
        .ok()?;
    candidats.into_iter().find_map(|id| {
        requete
            .query_row([id], |l| l.get::<_, Option<String>>(0))
            .ok()
            .flatten()
            .filter(|u| !u.is_empty())
    })
}

/// Tous les artworks connus d'une carte qui ne sont **pas** dans le classeur.
///
/// Complémentaire du scan : celui-ci ne voit que les artworks attestés par un
/// autre tirage du même set. Ici on part de la carte et on liste ses
/// illustrations, quelle que soit leur distribution.
///
/// Les entrées rendues sont des **anomalies synthétiques** : `id` vaut `None`,
/// elles ne sont pas en base, et [`corriger`] les traite exactement comme les
/// autres. C'est ce qui permet à l'écran « Modifier l'artwork… » de mélanger
/// les deux origines sans les distinguer.
///
/// Rend une liste vide plutôt qu'une erreur : cette fonction sert à peupler
/// un choix, et un choix vide se présente très bien.
///
/// # Comment la carte est retrouvée — et le défaut du 2026-09-20
///
/// L'identité cherchée est le `card_uuid` d'YGOJSON. La première version ne
/// connaissait que cette voie, et **sortait les mains vides quand la colonne
/// était vide**. Or le chemin de création par YGOPRODeck n'invente pas
/// d'identifiant YGOJSON — la colonne y est vide par construction, et c'est
/// voulu (cf. [`crate::creation::LigneClasseur`]).
///
/// Mesuré sur l'installation de l'utilisateur : **onze classeurs sur seize**
/// avaient `card_uuid` vide sur la totalité de leurs lignes. Pour ceux-là,
/// « Modifier l'artwork… » ne pouvait **jamais** rien proposer, quoi que
/// `cardinfo.db` contienne. 88 propositions sur l'ensemble de l'installation.
///
/// Le repli emprunte l'autre identifiant, celui qu'YGOPRODeck donne toujours :
/// `card_image_id` → `card_images.ygoprodeck_image_id` → `card_uuid`. La même
/// carte, par l'autre bout. Après repli : **254 propositions**, et les onze
/// classeurs répondent — MP25 passe de 0 à 58, LDK2 de 0 à 41, SDWD de 0 à 24.
///
/// Le repli ne s'active que si la voie directe échoue : un classeur qui porte
/// ses `card_uuid` ne change pas de comportement, ce que les mesures
/// confirment classeur par classeur.
#[must_use]
pub fn artworks_alternatifs(paths: &Paths, classeur: &str, set_code: &str) -> Vec<Anomalie> {
    let Ok(base) = ygo_db::connexion::ouvrir_lecture_seule(paths.classeur_db(classeur)) else {
        return Vec::new();
    };
    let identite: Option<(String, String, String)> = base
        .query_row(
            "SELECT COALESCE(card_uuid,''), COALESCE(name,''), COALESCE(rarity,'')
               FROM cards
              WHERE set_code = ?1
              ORDER BY (card_uuid IS NULL OR card_uuid = '')
              LIMIT 1",
            [set_code],
            |l| Ok((l.get(0)?, l.get(1)?, l.get(2)?)),
        )
        .ok();
    let Some((card_uuid, nom, rarete)) = identite else {
        return Vec::new();
    };
    let deja: HashSet<i64> = base
        .prepare(
            "SELECT card_image_id FROM cards WHERE set_code = ?1 AND card_image_id IS NOT NULL",
        )
        .and_then(|mut r| {
            r.query_map([set_code], |l| l.get(0))
                .and_then(Iterator::collect::<rusqlite::Result<HashSet<i64>>>)
        })
        .unwrap_or_default();

    let Ok(cardinfo) = ygo_db::connexion::ouvrir_lecture_seule(paths.cardinfo_db()) else {
        return Vec::new();
    };
    // Le repli : sans `card_uuid`, l'identité se retrouve par l'identifiant
    // d'illustration, que les deux chemins de création écrivent toujours.
    let card_uuid = if card_uuid.is_empty() {
        match uuid_par_illustration(&cardinfo, &deja) {
            Some(uuid) => uuid,
            None => return Vec::new(),
        }
    } else {
        card_uuid
    };
    let Ok(mut requete) = cardinfo.prepare(
        "SELECT uuid, ygoprodeck_image_id, card_url, art_url
           FROM card_images WHERE card_uuid = ?1 ORDER BY ygoprodeck_image_id",
    ) else {
        return Vec::new();
    };
    let tous: Vec<(String, Option<i64>, String, String)> = requete
        .query_map([&card_uuid], |l| {
            Ok((
                l.get::<_, Option<String>>(0)?.unwrap_or_default(),
                l.get(1)?,
                l.get::<_, Option<String>>(2)?.unwrap_or_default(),
                l.get::<_, Option<String>>(3)?.unwrap_or_default(),
            ))
        })
        .and_then(Iterator::collect::<rusqlite::Result<Vec<_>>>)
        .unwrap_or_default();

    tous.into_iter()
        .enumerate()
        .filter_map(|(rang, (uuid, image_id, url, small))| {
            // Sans identifiant, ni l'affichage ni la correction ne savent quoi
            // faire de l'artwork. Le Python l'ignore aussi.
            let id = image_id?;
            if deja.contains(&id) {
                return None;
            }
            Some(Anomalie {
                id: None,
                nom: nom.clone(),
                art_a_image_uuid: String::new(),
                art_b_image_uuid: uuid,
                art_index: u32::try_from(rang + 1).unwrap_or(1),
                set_code_prefix: prefixe_classeur(set_code),
                missing_set_code: set_code.to_owned(),
                missing_set_rarity: rarete.clone(),
                image_url: url,
                image_url_small: small,
                image_id: Some(id),
                corrige: false,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    fn ligne(nom: &str, uuid: &str, code: &str, rarete: &str, id: i64) -> LigneScan {
        LigneScan {
            nom: nom.to_owned(),
            image_uuid: uuid.to_owned(),
            set_code: code.to_owned(),
            rarity: rarete.to_owned(),
            image_id: Some(id),
            image_url: format!("https://images/{id}.jpg"),
            image_url_small: format!("https://images/petit/{id}.jpg"),
        }
    }

    fn classeurs(codes: &[&str]) -> HashSet<String> {
        codes.iter().map(|c| (*c).to_owned()).collect()
    }

    /// Le préfixe garde le suffixe OCG et jette le code-langue TCG.
    #[test]
    fn le_prefixe_distingue_l_ocg_du_tcg() {
        assert_eq!(prefixe_classeur("CROS-EN001"), "CROS");
        assert_eq!(prefixe_classeur("RA02-EU006"), "RA02");
        assert_eq!(prefixe_classeur("LOCH-JP001"), "LOCH-JP");
        assert_eq!(prefixe_classeur("CROS-JP002"), "CROS-JP");
        assert_eq!(prefixe_classeur("L26D-ENM01"), "L26D");
        assert_eq!(prefixe_classeur("lob-en1"), "LOB", "la casse est ramenée");
        assert_eq!(prefixe_classeur("PROMO"), "PROMO");
        assert_eq!(prefixe_classeur(""), "");
    }

    /// Une carte à un seul artwork n'a rien à comparer.
    #[test]
    fn un_seul_artwork_ne_produit_aucune_anomalie() {
        let lignes = vec![
            ligne("Dark Magician", "a", "RA02-EN001", "Ultra Rare", 1),
            ligne("Dark Magician", "a", "RA02-EN001", "Secret Rare", 1),
        ];
        assert!(detecter(&lignes, &classeurs(&["RA02"])).is_empty());
    }

    /// Le cœur : deux artworks, un tirage que seul le premier atteste.
    #[test]
    fn un_artwork_absent_d_un_tirage_est_une_anomalie() {
        let lignes = vec![
            ligne("Droll & Lock Bird", "aaa", "RA02-EN006", "Ultra Rare", 10),
            ligne("Droll & Lock Bird", "aaa", "MP01-EN001", "Ultra Rare", 10),
            // Le second artwork n'existe que dans l'autre set.
            ligne("Droll & Lock Bird", "bbb", "MP01-EN001", "Ultra Rare", 20),
        ];
        let a = detecter(&lignes, &classeurs(&["RA02"]));
        assert_eq!(a.len(), 1, "{a:?}");
        assert_eq!(a[0].nom, "Droll & Lock Bird");
        assert_eq!(a[0].missing_set_code, "RA02-EN006");
        assert_eq!(a[0].missing_set_rarity, "Ultra Rare");
        assert_eq!(a[0].art_a_image_uuid, "aaa");
        assert_eq!(a[0].art_b_image_uuid, "bbb");
        assert_eq!(
            a[0].image_id,
            Some(20),
            "l'illustration à poser est celle de B"
        );
        assert_eq!(a[0].set_code_prefix, "RA02");
        assert!(!a[0].corrige);
    }

    /// La comparaison va dans **les deux sens**.
    ///
    /// # Le défaut que ce test fige
    ///
    /// Le Python ne comparait d'abord que du premier uuid vers les suivants.
    /// Sur *Droll & Lock Bird*, l'Art 2 porte un uuid alphabétiquement
    /// **inférieur** à celui de l'Art 1 : il devenait le « A », et la seule
    /// direction utile — `RA02` a l'Art 1, pas l'Art 2 — n'était jamais
    /// calculée.
    #[test]
    fn la_comparaison_va_dans_les_deux_sens() {
        let lignes = vec![
            // `486a` < `56b5` : l'artwork au plus petit uuid est le second.
            ligne("Droll & Lock Bird", "486a", "MP01-EN001", "Ultra Rare", 20),
            ligne("Droll & Lock Bird", "56b5", "MP01-EN001", "Ultra Rare", 10),
            ligne("Droll & Lock Bird", "56b5", "RA02-EN006", "Ultra Rare", 10),
        ];
        let a = detecter(&lignes, &classeurs(&["RA02"]));
        assert_eq!(a.len(), 1, "la direction inverse compte aussi : {a:?}");
        assert_eq!(a[0].art_b_image_uuid, "486a");
        assert_eq!(a[0].missing_set_code, "RA02-EN006");
    }

    /// Une anomalie sur un set qu'on ne collectionne pas ne sort pas.
    #[test]
    fn seuls_les_classeurs_de_l_installation_comptent() {
        let lignes = vec![
            ligne("Dark Magician", "aaa", "XYZ-EN001", "Common", 1),
            ligne("Dark Magician", "aaa", "MP01-EN001", "Common", 1),
            ligne("Dark Magician", "bbb", "MP01-EN001", "Common", 2),
        ];
        assert!(detecter(&lignes, &classeurs(&["RA02"])).is_empty());
        assert_eq!(detecter(&lignes, &classeurs(&["XYZ"])).len(), 1);
    }

    /// Trois artworks A qui attestent le même tirage ne donnent qu'une
    /// anomalie : la clé de la table est `(art_b, code, rareté)`.
    #[test]
    fn le_meme_manque_n_est_compte_qu_une_fois() {
        let lignes = vec![
            ligne("Blue-Eyes", "a1", "RA02-EN001", "Common", 1),
            ligne("Blue-Eyes", "a2", "RA02-EN001", "Common", 2),
            ligne("Blue-Eyes", "a3", "RA02-EN001", "Common", 3),
            // `zz` n'est nulle part sur RA02-EN001 : trois A l'accusent.
            ligne("Blue-Eyes", "zz", "MP01-EN001", "Common", 9),
        ];
        let a = detecter(&lignes, &classeurs(&["RA02"]));
        let manquants: Vec<&str> = a.iter().map(|x| x.art_b_image_uuid.as_str()).collect();
        assert_eq!(
            manquants.iter().filter(|u| **u == "zz").count(),
            1,
            "une seule ligne pour `zz` : {manquants:?}"
        );
    }

    /// Le tri suit l'ordre du classeur : numéro, puis rareté, puis nom — et
    /// les sous-decks se groupent.
    #[test]
    fn le_tri_suit_l_ordre_du_classeur() {
        let fabrique = |code: &str, rarete: &str, nom: &str| Anomalie {
            id: None,
            nom: nom.to_owned(),
            art_a_image_uuid: String::new(),
            art_b_image_uuid: String::new(),
            art_index: 1,
            set_code_prefix: prefixe_classeur(code),
            missing_set_code: code.to_owned(),
            missing_set_rarity: rarete.to_owned(),
            image_url: String::new(),
            image_url_small: String::new(),
            image_id: None,
            corrige: false,
        };
        let mut v = vec![
            fabrique("RA02-EN010", "Common", "Z"),
            fabrique("RA02-EN002", "Ultra Rare", "A"),
            fabrique("RA02-EN002", "Common", "A"),
            fabrique("L26D-ENS01", "Common", "S"),
            fabrique("L26D-ENM01", "Common", "M"),
        ];
        trier(&mut v);
        let ordre: Vec<&str> = v.iter().map(|a| a.missing_set_code.as_str()).collect();
        assert_eq!(
            ordre,
            vec![
                "L26D-ENM01",
                "L26D-ENS01",
                "RA02-EN002",
                "RA02-EN002",
                "RA02-EN010"
            ],
            "les sous-decks se groupent, et 2 passe avant 10"
        );
        assert_eq!(v[2].missing_set_rarity, "Common", "puis la rareté");

        // Un code sans numéro exploitable ne fait pas paniquer le tri.
        let mut bancal = vec![fabrique("PROMO", "Common", "P")];
        trier(&mut bancal);
        assert_eq!(cle_de_tri(&bancal[0]).2, 0);
    }

    /// Une installation jetable : une `cardinfo.db` minimale et un classeur
    /// qui porte l'Art A d'une carte.
    fn installation() -> (tempfile::TempDir, Paths) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(tmp.path());
        paths.creer_dossiers().unwrap();

        // `cardinfo.db` : les quatre tables requises, peuplées du strict
        // nécessaire pour que le scan voie deux artworks.
        let conn = rusqlite::Connection::open(paths.cardinfo_db()).unwrap();
        conn.execute_batch(
            "CREATE TABLE set_prints (set_locale_id INTEGER, card_uuid TEXT,
                                      card_image_uuid TEXT, set_code TEXT, rarity TEXT);
             CREATE TABLE set_locales (id INTEGER PRIMARY KEY, language TEXT);
             CREATE TABLE card_texts (card_uuid TEXT, language TEXT, name TEXT);
             CREATE TABLE card_images (uuid TEXT PRIMARY KEY, card_uuid TEXT,
                                       ygoprodeck_image_id INTEGER, card_url TEXT, art_url TEXT);
             INSERT INTO set_locales VALUES (1, 'en');
             INSERT INTO card_texts VALUES ('c1', 'en', 'Droll & Lock Bird');
             INSERT INTO card_images VALUES ('artA', 'c1', 10, 'urlA', 'petitA');
             INSERT INTO card_images VALUES ('artB', 'c1', 20, 'urlB', 'petitB');
             INSERT INTO set_prints VALUES (1, 'c1', 'artA', 'RA02-EN006', 'Ultra Rare');
             INSERT INTO set_prints VALUES (1, 'c1', 'artA', 'MP01-EN001', 'Ultra Rare');
             INSERT INTO set_prints VALUES (1, 'c1', 'artB', 'MP01-EN001', 'Ultra Rare');",
        )
        .unwrap();
        drop(conn);

        // Le classeur RA02, avec la seule ligne de l'Art A.
        std::fs::create_dir_all(paths.dossier_classeur("RA02")).unwrap();
        let classeur = rusqlite::Connection::open(paths.classeur_db("RA02")).unwrap();
        classeur
            .execute_batch(ygo_db::schema::DDL_CLASSEUR)
            .unwrap();
        classeur
            .execute(
                "INSERT INTO cards (card_uuid, card_image_uuid, set_code, rarity, rarity_code,
                                    set_name, name, name_fr, card_image_url, card_image_small,
                                    card_image_id, sort_order, card_type, atk, def_val, level,
                                    attribute, race, possessed, quantite)
                 VALUES ('c1','artA','RA02-EN006','Ultra Rare','UR','Rarity Collection II',
                         'Droll & Lock Bird','Droll et Lock Bird','urlA','petitA',10,5,
                         'Effect Monster',0,0,1,'WIND','Winged Beast',0,0)",
                [],
            )
            .unwrap();
        (tmp, paths)
    }

    /// Le chemin complet : scanner, lire, corriger, annuler.
    #[test]
    fn le_chemin_complet_passe_par_les_vraies_bases() {
        let (_tmp, paths) = installation();
        assert!(
            tables_manquantes(&paths).is_empty(),
            "cardinfo est complète"
        );

        let bilan = scanner(&paths).unwrap();
        assert_eq!(bilan.detectees, 1);
        assert_eq!(bilan.ajoutees, 1);
        assert_eq!(bilan.total, 1);

        // Rescanner ne duplique rien — la clé unique de la table tient.
        let encore = scanner(&paths).unwrap();
        assert_eq!(encore.ajoutees, 0, "INSERT OR IGNORE");
        assert_eq!(encore.total, 1);

        let anomalies = lire(&paths, None).unwrap();
        assert_eq!(anomalies.len(), 1);
        let a = &anomalies[0];
        assert!(a.id.is_some(), "elle vient de la base");
        assert_eq!(a.missing_set_code, "RA02-EN006");
        assert_eq!(a.image_id, Some(20), "l'Art B");
        assert!(!a.corrige);
        assert_eq!(
            classeurs_avec_anomalies(&paths).unwrap(),
            vec!["RA02".to_owned()]
        );

        // ── La correction ────────────────────────────────────────────────
        let correction = corriger(&paths, a).unwrap();
        let Correction::Ajoutee { classeur, rowid } = correction else {
            panic!("attendu une ligne ajoutée, obtenu {correction:?}");
        };
        assert_eq!(classeur, "RA02");

        let base = rusqlite::Connection::open(paths.classeur_db("RA02")).unwrap();
        let (uuid, id, url, sort, rarete, nom_fr, possede): (
            String,
            i64,
            String,
            i64,
            String,
            String,
            i64,
        ) = base
            .query_row(
                "SELECT card_image_uuid, card_image_id, card_image_url, sort_order,
                        rarity, name_fr, possessed
                   FROM cards WHERE rowid = ?1",
                [rowid],
                |l| {
                    Ok((
                        l.get(0)?,
                        l.get(1)?,
                        l.get(2)?,
                        l.get(3)?,
                        l.get(4)?,
                        l.get(5)?,
                        l.get(6)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(uuid, "artB");
        assert_eq!(id, 20);
        assert_eq!(url, "urlB", "l'illustration est celle de l'Art B");
        assert_eq!(
            sort, 5,
            "le rang de tri est hérité : les deux arts voisinent"
        );
        assert_eq!(rarete, "Ultra Rare", "la rareté aussi");
        assert_eq!(nom_fr, "Droll et Lock Bird");
        assert_eq!(
            possede, 0,
            "ajouter un artwork ne dit rien de la possession"
        );

        // Le drapeau a suivi, et rejouer ne double pas la ligne.
        assert!(lire(&paths, None).unwrap()[0].corrige);
        assert_eq!(
            corriger(&paths, a).unwrap(),
            Correction::DejaPresente,
            "idempotente"
        );
        let lignes: i64 = base
            .query_row("SELECT COUNT(*) FROM cards", [], |l| l.get(0))
            .unwrap();
        assert_eq!(lignes, 2);

        // ── L'annulation ─────────────────────────────────────────────────
        assert!(annuler(&paths, a).unwrap());
        let lignes: i64 = base
            .query_row("SELECT COUNT(*) FROM cards", [], |l| l.get(0))
            .unwrap();
        assert_eq!(lignes, 1, "la ligne ajoutée est repartie");
        assert!(
            !lire(&paths, None).unwrap()[0].corrige,
            "le drapeau retombe"
        );
    }

    /// Une ligne devenue possédée ne s'annule pas.
    ///
    /// # Un écart assumé au portage
    ///
    /// Le Python supprime sans regarder la quantité. Si l'utilisateur a coché
    /// l'artwork alternatif depuis la correction, « annuler » lui faisait
    /// perdre une possession — sans rien lui dire. La quantité est ici une
    /// garde : elle atteste que la ligne n'est plus seulement une proposition.
    #[test]
    fn une_ligne_possedee_ne_s_annule_pas() {
        let (_tmp, paths) = installation();
        scanner(&paths).unwrap();
        let a = lire(&paths, None).unwrap().remove(0);
        assert!(corriger(&paths, &a).unwrap().a_ecrit());

        let base = rusqlite::Connection::open(paths.classeur_db("RA02")).unwrap();
        base.execute(
            "UPDATE cards SET possessed = 1, quantite = 1 WHERE card_image_id = 20",
            [],
        )
        .unwrap();

        assert!(!annuler(&paths, &a).unwrap(), "rien n'est supprimé");
        let lignes: i64 = base
            .query_row("SELECT COUNT(*) FROM cards", [], |l| l.get(0))
            .unwrap();
        assert_eq!(lignes, 2);
    }

    /// Une rareté que le classeur n'a pas se crée depuis une autre du même
    /// numéro.
    ///
    /// # Le cas réel que ce test ferme
    ///
    /// `RA05-EN072` et `RA05-EN080` en `Common` : le classeur ne porte ces
    /// cartes qu'en Collector's, Platinum Secret, Secret, Starlight, Super et
    /// Ultimate. La carte est là, sa rareté Common n'y est pas, et la source
    /// dit qu'elle existe. Exiger un témoin de rareté identique laissait ces
    /// deux propositions sans réponse — 171 posées sur 173.
    #[test]
    fn une_rarete_absente_se_cree_depuis_une_autre_du_meme_numero() {
        let (_tmp, paths) = installation();
        scanner(&paths).unwrap();
        let mut a = lire(&paths, None).unwrap().remove(0);
        a.missing_set_rarity = "Secret Rare".to_owned();

        let correction = corriger(&paths, &a).unwrap();
        assert!(correction.a_ecrit(), "{correction:?}");
        let Correction::Ajoutee { rowid, .. } = correction else {
            panic!("attendu une ligne ajoutée")
        };

        let base = rusqlite::Connection::open(paths.classeur_db("RA02")).unwrap();
        let (rarete, code, image, nom): (String, Option<String>, i64, String) = base
            .query_row(
                "SELECT rarity, rarity_code, card_image_id, name FROM cards WHERE rowid = ?1",
                [rowid],
                |l| Ok((l.get(0)?, l.get(1)?, l.get(2)?, l.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            rarete, "Secret Rare",
            "la rareté visée, pas celle du témoin"
        );
        assert_eq!(
            code.as_deref(),
            Some("ScR"),
            "le code d'YGOPRODeck — celui que la colonne parle —, pas `SCR` de Scanflip"
        );
        assert_eq!(image, 20, "et l'illustration reste celle de l'Art B");
        assert_eq!(nom, "Droll & Lock Bird");
    }

    /// Sans aucune ligne de ce numéro, il n'y a rien d'où hériter.
    #[test]
    fn sans_ligne_du_meme_numero_la_correction_ne_fabrique_rien() {
        let (_tmp, paths) = installation();
        scanner(&paths).unwrap();
        let mut a = lire(&paths, None).unwrap().remove(0);
        a.missing_set_code = "RA02-EN999".to_owned();
        assert_eq!(corriger(&paths, &a).unwrap(), Correction::SansTemoin);

        a.set_code_prefix = "INCONNU".to_owned();
        assert_eq!(corriger(&paths, &a).unwrap(), Correction::ClasseurAbsent);
    }

    /// Un classeur recréé retrouve ses corrections.
    #[test]
    fn un_classeur_neuf_rejoue_les_anomalies_connues() {
        let (_tmp, paths) = installation();
        scanner(&paths).unwrap();
        assert_eq!(appliquer_sur_classeur_neuf(&paths, "RA02").unwrap(), 1);
        // Une seconde passe ne repose rien : la ligne est déjà là.
        assert_eq!(appliquer_sur_classeur_neuf(&paths, "RA02").unwrap(), 0);
    }

    /// `cardinfo.db` absente : le scan le dit, il ne panique pas.
    #[test]
    fn une_cardinfo_absente_est_nommee() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(tmp.path());
        paths.creer_dossiers().unwrap();
        assert_eq!(tables_manquantes(&paths).len(), TABLES_REQUISES.len());
        std::fs::create_dir_all(paths.dossier_classeur("RA02")).unwrap();
        let erreur = lignes_de_scan(&paths).unwrap_err().to_string();
        assert!(erreur.contains("set_prints"), "{erreur}");
        assert!(
            erreur.contains("MAJ BDD"),
            "et il dit quoi faire : {erreur}"
        );
    }

    /// Les artworks alternatifs d'une carte : ceux que le classeur n'a pas.
    #[test]
    fn les_artworks_alternatifs_excluent_ceux_deja_presents() {
        let (_tmp, paths) = installation();
        let propositions = artworks_alternatifs(&paths, "RA02", "RA02-EN006");
        assert_eq!(propositions.len(), 1, "l'Art A est déjà là");
        assert_eq!(propositions[0].image_id, Some(20));
        assert!(propositions[0].id.is_none(), "synthétique");
        assert_eq!(propositions[0].missing_set_rarity, "Ultra Rare");

        // Elles se corrigent comme les autres.
        assert!(corriger(&paths, &propositions[0]).unwrap().a_ecrit());
        assert!(artworks_alternatifs(&paths, "RA02", "RA02-EN006").is_empty());

        // Un tirage inconnu ne rend rien, et ne panique pas.
        assert!(artworks_alternatifs(&paths, "RA02", "RA02-EN999").is_empty());
        assert!(artworks_alternatifs(&paths, "INCONNU", "RA02-EN006").is_empty());
    }

    fn anomalie(code: &str, rarete: &str, art: &str, index: u32) -> Anomalie {
        Anomalie {
            id: None,
            nom: format!("Carte {code}"),
            art_a_image_uuid: "artA".to_owned(),
            art_b_image_uuid: art.to_owned(),
            art_index: index,
            set_code_prefix: prefixe_classeur(code),
            missing_set_code: code.to_owned(),
            missing_set_rarity: rarete.to_owned(),
            image_url: format!("https://images/{art}.jpg"),
            image_url_small: String::new(),
            image_id: Some(1),
            corrige: false,
        }
    }

    /// La même image sous deux noms est reconnue ; deux images différentes
    /// ne le sont pas ; un fichier absent ne permet pas de conclure.
    #[test]
    fn deux_fichiers_identiques_se_reconnaissent_a_leur_contenu() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("100438021.jpg"), b"la meme image").unwrap();
        std::fs::write(tmp.path().join("68337209.jpg"), b"la meme image").unwrap();
        std::fs::write(tmp.path().join("autre.jpg"), b"une autre image").unwrap();

        assert_eq!(
            images_identiques(tmp.path(), "100438021.jpg", "68337209.jpg"),
            Some(true),
            "le doublon de LOCR-JP : deux identifiants, une image"
        );
        assert_eq!(
            images_identiques(tmp.path(), "68337209.jpg", "autre.jpg"),
            Some(false)
        );
        assert_eq!(
            images_identiques(tmp.path(), "68337209.jpg", "absent.jpg"),
            None
        );
        assert_eq!(
            images_identiques(tmp.path(), "absent.jpg", "absent.jpg"),
            Some(true),
            "un même nom désigne un même fichier"
        );
    }

    /// La forme exacte de `LOCR-JP` : quinze lignes, quatre décisions.
    #[test]
    fn quinze_lignes_deviennent_quatre_decisions() {
        let mut entree = Vec::new();
        for r in ["Prismatic Secret Rare", "Secret Rare", "Ultra Rare"] {
            entree.push(anomalie("LOCR-JP038", r, "maliss2", 2));
        }
        for code in ["LOCR-JP073", "LOCR-JP074", "LOCR-JP076"] {
            for r in [
                "Prismatic Secret Rare",
                "Secret Rare",
                "Super Rare",
                "Ultimate Rare",
            ] {
                entree.push(anomalie(code, r, &format!("{code}-b"), 2));
            }
        }
        let numeros = regrouper(entree);
        assert_eq!(numeros.len(), 4);
        assert_eq!(numeros.iter().map(Numero::restantes).sum::<usize>(), 15);
        assert_eq!(
            numeros[0].set_code, "LOCR-JP038",
            "l'ordre d'entrée est gardé"
        );
        assert_eq!(numeros[0].raretes.len(), 3);
        assert_eq!(numeros[0].artworks.len(), 1);
        assert_eq!(numeros[1].raretes.len(), 4);
    }

    /// Plusieurs artworks pour une même rareté : chacun a sa proposition, et
    /// ils se rangent par rang, quel que soit l'ordre d'arrivée.
    #[test]
    fn plusieurs_artworks_se_choisissent_separement() {
        let entree = vec![
            anomalie("RA04-EN106", "Platinum Secret Rare", "art7", 7),
            anomalie("RA04-EN106", "Quarter Century Secret Rare", "art2", 2),
            anomalie("RA04-EN106", "Platinum Secret Rare", "art2", 2),
            anomalie("RA04-EN106", "Quarter Century Secret Rare", "art7", 7),
        ];
        let numeros = regrouper(entree);
        assert_eq!(numeros.len(), 1);
        let n = &numeros[0];
        assert_eq!(n.artworks.len(), 2);
        assert_eq!(n.artworks[0].art_index, 2, "rangés par rang");
        assert_eq!(n.artworks[1].art_index, 7);
        assert_eq!(
            n.raretes,
            vec!["Platinum Secret Rare", "Quarter Century Secret Rare"]
        );
        // Chaque artwork porte une proposition par rareté, dans l'ordre des
        // raretés du numéro.
        for p in &n.artworks {
            assert_eq!(p.par_rarete[0].missing_set_rarity, "Platinum Secret Rare");
            assert_eq!(
                p.par_rarete[1].missing_set_rarity,
                "Quarter Century Secret Rare"
            );
            assert!(p.pour("Quarter Century Secret Rare").is_some());
        }
    }

    /// Un artwork proposé pour une partie des raretés seulement n'en invente
    /// pas pour les autres.
    #[test]
    fn un_artwork_ne_couvre_que_ses_raretes() {
        let numeros = regrouper(vec![
            anomalie("RA02-EN009", "Secret Rare", "a2", 2),
            anomalie("RA02-EN009", "Ultra Rare", "a3", 3),
        ]);
        let n = &numeros[0];
        assert_eq!(n.raretes.len(), 2);
        assert!(n.artworks[0].pour("Ultra Rare").is_none());
        assert!(n.artworks[1].pour("Secret Rare").is_none());
    }

    /// Le même numéro dans deux classeurs reste deux décisions.
    #[test]
    fn deux_classeurs_ne_se_melangent_pas() {
        let mut a = anomalie("RA02-EN001", "Common", "x", 2);
        let mut b = a.clone();
        a.set_code_prefix = "RA02".to_owned();
        b.set_code_prefix = "AUTRE".to_owned();
        assert_eq!(regrouper(vec![a, b]).len(), 2);
    }

    /// Le défaut du 2026-09-20 : un classeur créé par le chemin YGOPRODeck
    /// n'a pas de `card_uuid`, et « Modifier l'artwork… » n'y proposait
    /// jamais rien.
    ///
    /// Onze classeurs sur seize étaient dans ce cas sur l'installation réelle.
    #[test]
    fn un_classeur_sans_card_uuid_propose_quand_meme_ses_artworks() {
        let (_tmp, paths) = installation();
        // Ce que le chemin API écrit : l'identifiant d'illustration, jamais
        // celui d'YGOJSON.
        let classeur = rusqlite::Connection::open(paths.classeur_db("RA02")).unwrap();
        classeur
            .execute(
                "UPDATE cards SET card_uuid = '' WHERE set_code = 'RA02-EN006'",
                [],
            )
            .unwrap();
        drop(classeur);

        let propositions = artworks_alternatifs(&paths, "RA02", "RA02-EN006");
        assert_eq!(
            propositions.len(),
            1,
            "l'identité se retrouve par card_image_id"
        );
        assert_eq!(propositions[0].image_id, Some(20));
        assert_eq!(propositions[0].nom, "Droll & Lock Bird");
        assert_eq!(propositions[0].missing_set_rarity, "Ultra Rare");
        assert!(corriger(&paths, &propositions[0]).unwrap().a_ecrit());
    }

    /// Le repli ne sert qu'en dernier recours : un classeur qui porte ses
    /// `card_uuid` garde exactement le comportement d'avant.
    #[test]
    fn le_repli_ne_change_rien_quand_l_identifiant_est_la() {
        let (_tmp, paths) = installation();
        let avec = artworks_alternatifs(&paths, "RA02", "RA02-EN006");

        let (_tmp2, paths2) = installation();
        let classeur = rusqlite::Connection::open(paths2.classeur_db("RA02")).unwrap();
        classeur
            .execute("UPDATE cards SET card_uuid = ''", [])
            .unwrap();
        drop(classeur);
        let sans = artworks_alternatifs(&paths2, "RA02", "RA02-EN006");

        assert_eq!(avec.len(), sans.len());
        assert_eq!(avec[0].image_id, sans[0].image_id);
        assert_eq!(avec[0].art_b_image_uuid, sans[0].art_b_image_uuid);
    }

    /// Un artwork externe porte un identifiant **négatif** et ne correspond à
    /// aucune ligne de `card_images` : le repli doit l'ignorer plutôt que de
    /// le chercher.
    #[test]
    fn le_repli_ignore_les_identifiants_d_artwork_externe() {
        let (_tmp, paths) = installation();
        let classeur = rusqlite::Connection::open(paths.classeur_db("RA02")).unwrap();
        classeur
            .execute(
                "UPDATE cards SET card_uuid = '', card_image_id = -42
                  WHERE set_code = 'RA02-EN006'",
                [],
            )
            .unwrap();
        drop(classeur);

        assert!(
            artworks_alternatifs(&paths, "RA02", "RA02-EN006").is_empty(),
            "rien à quoi se raccrocher — et surtout pas une panique"
        );
    }

    /// L'aperçu et la carte posée partagent le **même** fichier.
    ///
    /// C'est ce qui fait qu'un aperçu n'est pas un cache jetable : c'est
    /// l'image définitive, obtenue plus tôt. Une illustration déjà
    /// téléchargée pour un autre classeur s'affiche sans une requête.
    #[test]
    fn l_apercu_vise_le_fichier_de_la_carte() {
        // YGOPRODeck : l'identifiant fait l'URL, donc le nom du fichier.
        assert_eq!(
            fichier_de(
                "https://ailleurs/xyz.jpg",
                Some(89_631_139),
                SourceImage::Ygoprodeck
            ),
            Some("89631139.jpg".to_owned())
        );
        // Yugipedia : l'URL stockée telle quelle.
        assert_eq!(
            fichier_de(
                "https://ms.yugipedia.com//4/4e/Maiden-SDWD.png",
                Some(1),
                SourceImage::Yugipedia
            ),
            Some("Maiden-SDWD.png".to_owned())
        );
        // Un artwork externe garde son URL quelle que soit la source.
        assert_eq!(
            fichier_de("https://ext/a.png", Some(-42), SourceImage::Ygoprodeck),
            Some("a.png".to_owned())
        );
        assert_eq!(fichier_de("", None, SourceImage::Ygoprodeck), None);
    }

    /// Une illustration déjà sur disque ne se retélécharge pas.
    #[test]
    fn un_apercu_deja_la_ne_se_redemande_pas() {
        let (_tmp, paths) = installation();
        scanner(&paths).unwrap();
        let a = lire(&paths, None).unwrap().remove(0);

        let Some(cible) = apercu_a_telecharger(&paths, &a, SourceImage::Ygoprodeck) else {
            panic!("l'image n'est pas encore là : une cible devait être rendue")
        };
        assert_eq!(cible.destination, paths.img_small().join("20.jpg"));
        assert!(cible.url_primaire.contains("20.jpg"));

        std::fs::create_dir_all(paths.img_small()).unwrap();
        std::fs::write(&cible.destination, b"une image").unwrap();
        assert!(
            apercu_a_telecharger(&paths, &a, SourceImage::Ygoprodeck).is_none(),
            "présente : plus rien à demander"
        );
    }

    /// L'illustration que le classeur porte déjà, par tirage.
    #[test]
    fn les_artworks_en_place_se_lisent_par_tirage() {
        let (_tmp, paths) = installation();
        let en_place = artworks_en_place(&paths, "RA02", SourceImage::Ygoprodeck);
        assert_eq!(
            en_place.get(&("RA02-EN006".to_owned(), "Ultra Rare".to_owned())),
            Some(&"10.jpg".to_owned()),
            "l'Art A, celui qui est dans le classeur"
        );
        assert!(artworks_en_place(&paths, "INCONNU", SourceImage::Ygoprodeck).is_empty());
    }

    /// Les propositions d'une carte réunissent les deux origines sans se
    /// répéter.
    #[test]
    fn les_propositions_d_une_carte_reunissent_les_deux_origines() {
        let (_tmp, paths) = installation();
        scanner(&paths).unwrap();

        // Le scan connaît l'Art B ; `card_images` aussi. Une seule ligne.
        let p = propositions_pour_carte(&paths, "RA02", "RA02-EN006").unwrap();
        assert_eq!(p.len(), 1, "{p:?}");
        assert_eq!(p[0].image_id, Some(20));
        assert!(p[0].id.is_some(), "celle du scan gagne, elle est en base");

        // Un troisième artwork que le scan ne peut pas voir — aucun autre set
        // ne l'atteste — vient compléter la liste.
        let cardinfo = rusqlite::Connection::open(paths.cardinfo_db()).unwrap();
        cardinfo
            .execute(
                "INSERT INTO card_images VALUES ('artC','c1',30,'urlC','petitC')",
                [],
            )
            .unwrap();
        let p = propositions_pour_carte(&paths, "RA02", "RA02-EN006").unwrap();
        assert_eq!(p.len(), 2);
        let ids: HashSet<Option<i64>> = p.iter().map(|a| a.image_id).collect();
        assert_eq!(ids, [Some(20), Some(30)].into_iter().collect());
        assert_eq!(
            p.iter().filter(|a| a.id.is_none()).count(),
            1,
            "la synthétique est la nouvelle"
        );
    }

    /// L'oracle : les 173 anomalies des neuf classeurs réels.
    ///
    /// L'entrée est l'extrait exact de `cardinfo.db` pour les 29 cartes
    /// concernées — 2 608 tirages. Restreindre aux noms utiles ne change
    /// rien au résultat : la détection travaille carte par carte.
    #[test]
    fn l_oracle_des_neuf_classeurs_reels() {
        #[derive(serde::Deserialize)]
        struct Entree {
            classeurs: Vec<String>,
            lignes: Vec<LigneBrute>,
        }
        #[derive(serde::Deserialize)]
        struct LigneBrute {
            nom: String,
            image_uuid: String,
            set_code: String,
            rarity: String,
            image_id: Option<i64>,
            image_url: String,
            image_url_small: String,
        }
        #[derive(serde::Deserialize, Debug, PartialEq, Eq)]
        struct Attendue {
            nom: String,
            art_a: String,
            art_b: String,
            art_index: u32,
            prefixe: String,
            code: String,
            rarete: String,
        }

        let entree: Entree = serde_json::from_str(include_str!(
            "../../../tests/fixtures/oracle/anomalies/anomalies_entree.json"
        ))
        .unwrap();
        let attendues: Vec<Attendue> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/oracle/anomalies/anomalies_attendu.json"
        ))
        .unwrap();

        let lignes: Vec<LigneScan> = entree
            .lignes
            .into_iter()
            .map(|b| LigneScan {
                nom: b.nom,
                image_uuid: b.image_uuid,
                set_code: b.set_code,
                rarity: b.rarity,
                image_id: b.image_id,
                image_url: b.image_url,
                image_url_small: b.image_url_small,
            })
            .collect();
        assert_eq!(lignes.len(), 2_608);

        let classeurs: HashSet<String> = entree.classeurs.into_iter().collect();
        let obtenues = detecter(&lignes, &classeurs);
        assert_eq!(obtenues.len(), 173, "le compte du Python, à l'unité");

        // Comparaison ensembliste : le Python ne trie pas comme nous avant
        // d'insérer, et l'ordre d'insertion n'a aucune valeur.
        let clef = |a: &Anomalie| {
            (
                a.nom.clone(),
                a.art_a_image_uuid.clone(),
                a.art_b_image_uuid.clone(),
                a.art_index,
                a.set_code_prefix.clone(),
                a.missing_set_code.clone(),
                a.missing_set_rarity.clone(),
            )
        };
        let notre: BTreeSet<_> = obtenues.iter().map(clef).collect();
        let leur: BTreeSet<_> = attendues
            .iter()
            .map(|a| {
                (
                    a.nom.clone(),
                    a.art_a.clone(),
                    a.art_b.clone(),
                    a.art_index,
                    a.prefixe.clone(),
                    a.code.clone(),
                    a.rarete.clone(),
                )
            })
            .collect();
        assert_eq!(notre, leur, "les 173 anomalies, à l'identique");

        // La répartition par classeur, telle que mesurée le 2026-09-01.
        let mut par_classeur: BTreeMap<&str, usize> = BTreeMap::new();
        for a in &obtenues {
            *par_classeur.entry(a.set_code_prefix.as_str()).or_default() += 1;
        }
        assert_eq!(
            par_classeur,
            [
                ("LOCR-JP", 15),
                ("RA02", 63),
                ("RA05", 82),
                ("SDWD", 10),
                ("VASM", 3)
            ]
            .into_iter()
            .collect::<BTreeMap<&str, usize>>()
        );

        // Et le résultat est trié : la première anomalie de `RA02` est celle
        // du plus petit numéro.
        let premier_ra02 = obtenues.iter().find(|a| a.set_code_prefix == "RA02");
        assert!(premier_ra02.is_some());
    }
}
