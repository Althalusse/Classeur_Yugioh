// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Les scans Yugipedia d'un tirage — ce que « Modifier l'artwork… » propose
//! d'abord.
//!
//! # Pourquoi — 2026-10-17
//!
//! Retour d'usage : « il n'a pas l'air de taper dans les artworks de
//! Yugipedia, celles déjà présentes en local, avant d'aller check l'API ; là,
//! dans l'immédiat, il tape dans l'autre API ». C'était exact. L'écran ne
//! connaissait que deux origines, YGOPRODeck toutes deux : les anomalies du
//! scan et les illustrations que `card_images` connaît pour la carte
//! ([`crate::anomalies::propositions_pour_carte`]). Ses aperçus étaient les
//! `images.ygoprodeck.com/<id>.jpg`, demandés dès l'ouverture, quel que soit
//! le réglage « Source des images ». Les 1 575 scans Yugipedia que
//! l'installation réelle garde dans `img/small` n'étaient jamais regardés.
//!
//! # D'où viennent les scans, et dans quel ordre
//!
//! 1. **ce qui est déjà là**, sans réseau ([`scans_locaux`]) : les images en
//!    place dans les lignes du numéro, les `print_image_url` de
//!    `cardinfo.db` (YGOJSON), et les fichiers de `img/small` ;
//! 2. **Yugipedia** ([`scans_distants`]) : une requête `allimages` sur le
//!    préfixe `<NomDeCarte>-<SET>-`, qui ramène les fichiers de cette carte
//!    dans ce set et eux seuls. Elle passe par le quota du client (une
//!    requête toutes les 1,1 s, une seule en vol) et par son cache disque :
//!    30 jours, la période que le wiki conseille
//!    ([`ygo_sources::cache::duree_de_cache`]). Rouvrir la carte ne coûte
//!    rien ;
//! 3. **YGOPRODeck** en dernier : c'est l'écran qui décide de ne montrer ses
//!    illustrations que si Yugipedia ne connaît aucun scan du tirage, ou sur
//!    demande ([`Tableau::connu`]).
//!
//! Le nom Yugipedia d'une carte est son nom anglais sans espaces ni
//! ponctuation : c'est le cas de 3 303 des 3 348 lignes qui portent un scan
//! dans les seize classeurs réels. Les 45 autres sont des cartes renommées
//! (`LOCH-JP013`) ou des jetons ; leur nom se lit alors sur les scans déjà en
//! place, qui le portent ([`Carte::segments`]).
//!
//! # Une colonne par illustration, une image par rareté
//!
//! Une illustration YGOPRODeck est la même image pour toutes les raretés. Un
//! scan Yugipedia est celui **d'un** tirage : sa rareté, son édition, son
//! illustration. Les colonnes se font donc sur le suffixe du nom
//! ([`ygo_sources::yugipedia::artwork::suffixe`]) — `""` pour le tirage nu,
//! `B`, `C`, `2`, `AA`, `EA`… — et chaque colonne a une image par rareté :
//! l'édition de la ligne d'abord, puis `1E`, sans édition, `UE`, `LE`.
//!
//! Une illustration est **en place** pour une rareté quand une ligne de cette
//! rareté porte un scan de même suffixe, quelle qu'en soit l'édition :
//! `…-UR-UE-C.png` en place couvre `…-UR-1E-C.png`. Une ligne qui montre une
//! image YGOPRODeck n'en couvre aucune — c'est le cas que
//! [`mettre_sur_la_carte`] règle, sans ajouter de ligne.
//!
//! # Un nom de fichier ne dit pas son numéro
//!
//! `DarkMagician-RA04-EN-QCScR-1E-AA2.png` nomme une carte, un set, une
//! rareté et une illustration — pas un numéro. Or `RA04` imprime Dark
//! Magician sous deux numéros (`RA04-EN001`, `RA04-EN106`), Blue-Eyes White
//! Dragon aussi (`EN002`, `EN108`), et `SDWD` trois fois Blue-Eyes. Relevé le
//! 2026-10-17 : sans précaution, `RA04-EN001` se voyait proposer les `AA2`,
//! `AA3`, `AA4` que la passe artworks a posés sur `RA04-EN106`.
//!
//! D'où la règle : une illustration **en place sur un autre numéro** du
//! classeur lui appartient, et n'est pas proposée ici ([`Carte::ailleurs`]) —
//! sauf si ce numéro la porte aussi. Seule la galerie du set saurait
//! trancher plus finement ; ce qui n'est porté nulle part reste proposé, et
//! l'utilisateur juge sur l'image.
//!
//! # Ce que le module ne fait pas
//!
//! - proposer une rareté que le classeur n'a pas pour ce numéro : c'est
//!   l'affaire de la passe artworks et des numéros absents ;
//! - proposer un fichier à jeton de contexte (`OP`, `DT`, `PR`…) : une autre
//!   photo du même tirage, pas une carte à collectionner ;
//! - écrire quoi que ce soit sans un geste de l'utilisateur.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::OptionalExtension;
use ygo_core::paths::Paths;
use ygo_sources::yugipedia::artwork;

use crate::anomalies::{self, Anomalie, ArtworkPropose, Correction, Origine};
use crate::artworks::colonne_existe;
use crate::error::Result;

/// Préfixe du `card_image_uuid` d'un scan — celui que la passe artworks pose.
pub const PREFIXE_SCAN: &str = "yugipedia:";

/// Préfixe de l'identité d'une colonne de scans (cf.
/// [`ArtworkPropose::image_uuid`]).
pub const PREFIXE_COLONNE: &str = "yugipedia-colonne:";

/// Une ligne du classeur pour le numéro regardé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LigneCarte {
    /// `rowid` de la ligne.
    pub rowid: i64,
    /// Rareté, telle qu'écrite dans le classeur.
    pub rarete: String,
    /// Tirage Overframe (`extended_art = 1`).
    pub overframe: bool,
    /// Édition Scanflip (`1st`, `unlimited`, `limited`), si renseignée.
    pub edition: Option<String>,
    /// Exemplaires possédés.
    pub quantite: i64,
    /// Adresse stockée.
    pub url: String,
    /// Identifiant d'image.
    pub image_id: Option<i64>,
    /// Le scan Yugipedia que la ligne porte, d'après son adresse — une
    /// révision archivée ramenée à son fichier
    /// ([`artwork::nom_courant`]).
    pub scan: Option<String>,
}

/// Une carte d'un classeur : un numéro, et toutes ses lignes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Carte {
    /// Le classeur.
    pub classeur: String,
    /// Le numéro, tel que les lignes l'écrivent.
    pub set_code: String,
    /// Le nom anglais de la carte.
    pub nom: String,
    /// Le préfixe de set (`LCKC`).
    pub prefixe: String,
    /// La langue des fichiers attendus (`EN`, `JP`…).
    pub langue: String,
    /// Les lignes du numéro, dans l'ordre du classeur.
    pub lignes: Vec<LigneCarte>,
    /// Les illustrations que **d'autres numéros** du classeur portent :
    /// `(clé de carte, rareté, suffixe)`. Un nom de fichier ne dit pas son
    /// numéro — cf. l'en-tête du module.
    pub ailleurs: BTreeSet<(String, String, String)>,
}

impl Carte {
    /// Les raretés du numéro, sans doublon, dans l'ordre des lignes.
    #[must_use]
    pub fn raretes(&self) -> Vec<String> {
        let mut raretes: Vec<String> = Vec::new();
        for l in &self.lignes {
            if !raretes.contains(&l.rarete) {
                raretes.push(l.rarete.clone());
            }
        }
        raretes
    }

    /// La ligne de ce `rowid`, si elle appartient au numéro.
    #[must_use]
    pub fn ligne(&self, rowid: i64) -> Option<&LigneCarte> {
        self.lignes.iter().find(|l| l.rowid == rowid)
    }

    /// Les noms sous lesquels Yugipedia peut écrire la carte : son nom sans
    /// espaces ni ponctuation, et ceux des scans déjà en place.
    ///
    /// ```
    /// use ygo_app::scans::{Carte, LigneCarte};
    /// let carte = Carte {
    ///     classeur: "LOCH-JP".into(),
    ///     set_code: "LOCH-JP013".into(),
    ///     nom: "Odd-Eyes Pendulum Dragon of the Four Heavenly Dragons".into(),
    ///     prefixe: "LOCH".into(),
    ///     langue: "JP".into(),
    ///     lignes: vec![LigneCarte {
    ///         rowid: 1,
    ///         rarete: "Ultra Rare".into(),
    ///         overframe: false,
    ///         edition: None,
    ///         quantite: 0,
    ///         url: String::new(),
    ///         image_id: None,
    ///         scan: Some("OddEyesPendulumDragonFourHeavenlyDragons-LOCH-JP-UR.png".into()),
    ///     }],
    ///     ailleurs: Default::default(),
    /// };
    /// assert_eq!(
    ///     carte.prefixes_api(),
    ///     vec![
    ///         "OddEyesPendulumDragonFourHeavenlyDragons-LOCH-".to_owned(),
    ///         "OddEyesPendulumDragonoftheFourHeavenlyDragons-LOCH-".to_owned(),
    ///     ]
    /// );
    /// ```
    #[must_use]
    pub fn segments(&self) -> BTreeSet<String> {
        let mut segments: BTreeSet<String> = BTreeSet::new();
        let compact: String = self
            .nom
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect();
        if !compact.is_empty() {
            segments.insert(compact);
        }
        for scan in self.lignes.iter().filter_map(|l| l.scan.as_deref()) {
            if let Some((segment, _)) = artwork::decouper_fichier(scan, &self.prefixe) {
                if segment.chars().all(|c| c.is_ascii_alphanumeric()) {
                    segments.insert(segment);
                }
            }
        }
        segments
    }

    /// Les clés de carte qu'un fichier doit porter pour être retenu.
    #[must_use]
    pub fn cles(&self) -> BTreeSet<String> {
        let mut cles: BTreeSet<String> = self
            .segments()
            .iter()
            .map(|s| artwork::cle_comparaison(s))
            .collect();
        cles.insert(artwork::cle_comparaison(&self.nom));
        cles.retain(|c| !c.is_empty());
        cles
    }

    /// Les préfixes `allimages` à demander à Yugipedia — un par nom, le
    /// plus souvent un seul.
    #[must_use]
    pub fn prefixes_api(&self) -> Vec<String> {
        if self.prefixe.is_empty() {
            return Vec::new();
        }
        self.segments()
            .into_iter()
            .map(|s| format!("{s}-{}-", self.prefixe))
            .collect()
    }
}

/// Le scan qu'une adresse désigne, s'il en est un.
fn scan_de(url: &str) -> Option<String> {
    if !ygo_sources::http::est_yugipedia(url) {
        return None;
    }
    let nom = ygo_images::plan::nom_de_fichier(url)?;
    Some(artwork::nom_courant(&nom).to_owned())
}

/// Lit une carte d'un classeur — `None` si le classeur ou le numéro
/// n'existe pas.
///
/// # Errors
///
/// Rend une erreur si le classeur est illisible.
pub fn lire_carte(paths: &Paths, classeur: &str, set_code: &str) -> Result<Option<Carte>> {
    let chemin = paths.classeur_db(classeur);
    if !chemin.is_file() {
        return Ok(None);
    }
    let conn = ygo_db::connexion::ouvrir_lecture_seule(&chemin)?;
    let ext = if colonne_existe(&conn, "extended_art")? {
        "COALESCE(extended_art,0)"
    } else {
        "0"
    };
    let edition = if colonne_existe(&conn, "edition")? {
        "edition"
    } else {
        "NULL"
    };
    let sql = format!(
        "SELECT rowid, COALESCE(name,''), COALESCE(set_code,''), COALESCE(rarity,''), \
                {ext}, {edition}, COALESCE(quantite,0), COALESCE(card_image_url,''), \
                card_image_id \
           FROM cards WHERE UPPER(COALESCE(set_code,'')) = ?1 ORDER BY rowid"
    );
    let mut requete = conn.prepare(&sql)?;
    let brutes = requete
        .query_map([set_code.trim().to_uppercase()], |l| {
            Ok((
                l.get::<_, i64>(0)?,
                l.get::<_, String>(1)?,
                l.get::<_, String>(2)?,
                LigneCarte {
                    rowid: l.get(0)?,
                    rarete: l.get(3)?,
                    overframe: l.get::<_, i64>(4)? == 1,
                    edition: l
                        .get::<_, Option<String>>(5)?
                        .filter(|e| !e.trim().is_empty()),
                    quantite: l.get(6)?,
                    url: l.get(7)?,
                    image_id: l.get(8)?,
                    scan: None,
                },
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let Some((_, nom, code, _)) = brutes.iter().find(|(_, nom, _, _)| !nom.is_empty()) else {
        return Ok(None);
    };
    let (nom, code) = (nom.clone(), code.clone());
    let prefixe = artwork::prefixe_set(&code);
    let langue = match artwork::langue_set_code(&code) {
        l if l.is_empty() => "EN".to_owned(),
        l => l,
    };
    let lignes = brutes
        .into_iter()
        .map(|(_, _, _, mut l)| {
            l.scan = scan_de(&l.url);
            l
        })
        .collect();

    // Ce que portent les autres numéros : un scan qui y est en place leur
    // appartient.
    let mut ailleurs: BTreeSet<(String, String, String)> = BTreeSet::new();
    let mut requete = conn.prepare(
        "SELECT COALESCE(card_image_url,'') FROM cards \
          WHERE UPPER(COALESCE(set_code,'')) <> ?1 AND card_image_url LIKE '%yugipedia%'",
    )?;
    let urls = requete
        .query_map([code.to_uppercase()], |l| l.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for fichier in urls.iter().filter_map(|u| scan_de(u)) {
        if let Some((segment, infos)) = artwork::decouper_fichier(&fichier, &prefixe) {
            ailleurs.insert((
                artwork::cle_comparaison(&segment),
                infos.rarete_abbr.to_ascii_uppercase(),
                artwork::suffixe(&fichier, &prefixe),
            ));
        }
    }

    Ok(Some(Carte {
        classeur: classeur.to_owned(),
        set_code: code,
        nom,
        prefixe,
        langue,
        lignes,
        ailleurs,
    }))
}

/// Un scan Yugipedia d'un tirage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scan {
    /// Nom du fichier, tel que dans son adresse — et sur le disque.
    pub fichier: String,
    /// Adresse pleine résolution.
    pub url: String,
    /// Abréviation de rareté, en majuscules (`PLSCR`).
    pub rarete: String,
    /// Édition (`1E`, `UE`, `LE`), vide si le nom n'en dit rien.
    pub edition: String,
    /// Ce qui distingue l'illustration
    /// ([`artwork::suffixe`]).
    pub suffixe: String,
    /// La clé du nom de carte que le fichier porte
    /// ([`artwork::cle_comparaison`]).
    pub cle: String,
}

/// Les scans retenus pour une carte, sans doublon : le premier venu gagne,
/// d'où l'ordre des sources.
struct Recueil<'a> {
    carte: &'a Carte,
    cles: BTreeSet<String>,
    scans: BTreeMap<String, Scan>,
}

impl<'a> Recueil<'a> {
    fn new(carte: &'a Carte) -> Self {
        Self {
            carte,
            cles: carte.cles(),
            scans: BTreeMap::new(),
        }
    }

    /// Retient un fichier s'il est un scan de **cette** carte, dans **ce**
    /// set et **cette** langue, sans jeton de contexte.
    fn ajouter(&mut self, fichier: &str, url: &str) {
        if fichier.is_empty() || url.is_empty() || self.scans.contains_key(fichier) {
            return;
        }
        let prefixe = &self.carte.prefixe;
        let Some((segment, infos)) = artwork::decouper_fichier(fichier, prefixe) else {
            return;
        };
        let cle = artwork::cle_comparaison(&segment);
        if !self.cles.contains(&cle)
            || infos.rarete_abbr.is_empty()
            || artwork::porte_un_contexte(fichier)
            || (!infos.langue.is_empty() && infos.langue != self.carte.langue)
        {
            return;
        }
        self.scans.insert(
            fichier.to_owned(),
            Scan {
                fichier: fichier.to_owned(),
                url: url.to_owned(),
                rarete: infos.rarete_abbr.to_ascii_uppercase(),
                edition: infos.edition,
                suffixe: artwork::suffixe(fichier, prefixe),
                cle,
            },
        );
    }

    fn scans(self) -> Vec<Scan> {
        self.scans.into_values().collect()
    }
}

/// Les scans que l'installation connaît **sans réseau**.
///
/// Dans l'ordre : les lignes du numéro, `set_prints.print_image_url`, puis
/// `img/small`. Un fichier du disque n'a que son nom ; son adresse se déduit
/// ([`artwork::url_fichier`]). Une base ou un dossier illisible ne fait que
/// retirer sa part : c'est une source parmi trois, pas une condition.
#[must_use]
pub fn scans_locaux(paths: &Paths, carte: &Carte) -> Vec<Scan> {
    let mut recueil = Recueil::new(carte);
    let lignes = carte.lignes.iter().map(|l| l.url.clone());
    for url in lignes.chain(tirages_yugipedia(paths, &carte.set_code)) {
        let Some(fichier) = scan_de(&url) else {
            continue;
        };
        // Une révision archivée est rendue à son fichier courant : c'est lui
        // qu'on propose, à sa propre adresse.
        let url = if ygo_images::plan::nom_de_fichier(&url).as_deref() == Some(fichier.as_str()) {
            url
        } else {
            artwork::url_fichier(&fichier)
        };
        recueil.ajouter(&fichier, &url);
    }
    let marqueur = format!("-{}-", carte.prefixe).to_ascii_uppercase();
    if let Ok(entrees) = std::fs::read_dir(paths.img_small()) {
        let mut noms: Vec<String> = entrees
            .filter_map(std::result::Result::ok)
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| n.to_ascii_uppercase().contains(&marqueur))
            .collect();
        noms.sort();
        for nom in noms {
            let url = artwork::url_fichier(&nom);
            recueil.ajouter(&nom, &url);
        }
    }
    recueil.scans()
}

/// Les `print_image_url` Yugipedia de `cardinfo.db` pour un numéro.
fn tirages_yugipedia(paths: &Paths, set_code: &str) -> Vec<String> {
    let Ok(conn) = ygo_db::connexion::ouvrir_lecture_seule(paths.cardinfo_db()) else {
        return Vec::new();
    };
    let Ok(mut requete) = conn.prepare(
        "SELECT DISTINCT COALESCE(print_image_url,'') FROM set_prints WHERE set_code = ?1",
    ) else {
        return Vec::new();
    };
    requete
        .query_map([set_code], |l| l.get::<_, String>(0))
        .and_then(Iterator::collect::<rusqlite::Result<Vec<_>>>)
        .unwrap_or_default()
        .into_iter()
        .filter(|u| ygo_sources::http::est_yugipedia(u))
        .collect()
}

/// Les scans que Yugipedia déclare pour la carte — une requête `allimages`
/// par nom ([`Carte::prefixes_api`]), le plus souvent une seule.
///
/// # Errors
///
/// Rend l'erreur du réseau : l'écran se rabat alors sur ce qu'il sait déjà.
pub async fn scans_distants(client: &ygo_sources::ClientHttp, carte: &Carte) -> Result<Vec<Scan>> {
    let mut recueil = Recueil::new(carte);
    for prefixe in carte.prefixes_api() {
        for f in artwork::lister_images(client, &prefixe).await? {
            // Le nom tel que l'adresse l'écrit, comme sur le disque.
            let fichier = ygo_images::plan::nom_de_fichier(&f.url).unwrap_or(f.nom);
            recueil.ajouter(&fichier, &f.url);
        }
    }
    Ok(recueil.scans())
}

/// Réunit deux récoltes : les scans locaux d'abord, ceux de Yugipedia
/// complètent.
#[must_use]
pub fn fusionner(locaux: Vec<Scan>, distants: &[Scan]) -> Vec<Scan> {
    let mut tous: BTreeMap<String, Scan> =
        locaux.into_iter().map(|s| (s.fichier.clone(), s)).collect();
    for s in distants {
        tous.entry(s.fichier.clone()).or_insert_with(|| s.clone());
    }
    tous.into_values().collect()
}

/// Ce que l'écran montre des scans d'un tirage.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tableau {
    /// Une colonne par illustration dont **au moins une rareté** reste à
    /// prendre, au format de la grille de l'écran. Les cases déjà dans le
    /// classeur y sont marquées `corrige`.
    pub colonnes: Vec<ArtworkPropose>,
    /// Les illustrations que le classeur a déjà pour toutes leurs raretés —
    /// l'écran les range, sauf celles qu'il vient d'ajouter lui-même et
    /// qu'on peut encore retirer.
    pub completes: Vec<ArtworkPropose>,
    /// Yugipedia connaît-il au moins un scan de ce tirage, en place ou non ?
    /// C'est ce qui relègue les illustrations YGOPRODeck.
    pub connu: bool,
    /// Les scans écartés parce qu'un **autre numéro** du classeur les porte.
    pub ecartes: usize,
}

impl Tableau {
    /// Les libellés des illustrations complètes (« Scan », « Scan B »…).
    #[must_use]
    pub fn libelles_completes(&self) -> Vec<String> {
        self.completes.iter().map(ArtworkPropose::libelle).collect()
    }
}

/// L'ordre des colonnes : le tirage nu, les illustrations d'un même tirage
/// (`B`, `C`, `2`…), puis `AA`, `ALT`, `EA`, et le reste.
fn ordre_colonne(suffixe: &str) -> (u8, u32, String) {
    let chiffres = |s: &str| s.parse::<u32>().unwrap_or(0);
    let famille = |prefixe: &str| suffixe.strip_prefix(prefixe).map(chiffres);
    let rang = match suffixe.as_bytes() {
        [] => Some((0, 0)),
        [lettre] if lettre.is_ascii_alphabetic() => {
            Some((1, u32::from(lettre.to_ascii_uppercase() - b'A') + 1))
        }
        c if (1..=2).contains(&c.len()) && c.iter().all(u8::is_ascii_digit) => {
            Some((1, chiffres(suffixe)))
        }
        _ => None,
    };
    let (groupe, n) = rang
        .or_else(|| famille("AA").map(|n| (2, n)))
        .or_else(|| famille("ALT").map(|n| (3, n)))
        .or_else(|| famille("EA").map(|n| (4, n)))
        .unwrap_or((5, 0));
    (groupe, n, suffixe.to_owned())
}

/// Le rang d'une édition de fichier, à choix égal : celle de la ligne, puis
/// `1E`, aucune, `UE`, `LE`.
fn rang_edition(edition: &str, voulue: Option<&str>) -> usize {
    if voulue.is_some_and(|v| v.eq_ignore_ascii_case(edition)) {
        return 0;
    }
    ["1E", "", "UE", "LE"]
        .iter()
        .position(|e| e.eq_ignore_ascii_case(edition))
        .map_or(9, |p| p + 1)
}

/// L'édition Scanflip d'une ligne, dans le vocabulaire des fichiers.
fn edition_fichier(edition: Option<&str>) -> Option<&'static str> {
    match edition.map(str::trim) {
        Some("1st") => Some("1E"),
        Some("unlimited") => Some("UE"),
        Some("limited") => Some("LE"),
        _ => None,
    }
}

/// Range les scans d'une carte en colonnes — **fonction pure**.
///
/// Un scan qu'un autre numéro du classeur porte déjà est écarté, à moins que
/// celui-ci ne le porte aussi : c'est la carte de l'autre numéro.
///
/// ```
/// use ygo_app::scans::{tableau, Carte, LigneCarte, Scan};
/// let ligne = |rowid: i64, scan: Option<&str>| LigneCarte {
///     rowid,
///     rarete: "Ultra Rare".into(),
///     overframe: false,
///     edition: None,
///     quantite: 0,
///     url: String::new(),
///     image_id: None,
///     scan: scan.map(str::to_owned),
/// };
/// let scan = |f: &str, suffixe: &str| Scan {
///     fichier: f.into(),
///     url: format!("https://ms.yugipedia.com//x/xy/{f}"),
///     rarete: "UR".into(),
///     edition: "1E".into(),
///     suffixe: suffixe.into(),
///     cle: "blueeyeswhitedragon".into(),
/// };
/// let mut carte = Carte {
///     classeur: "LCKC".into(),
///     set_code: "LCKC-EN001".into(),
///     nom: "Blue-Eyes White Dragon".into(),
///     prefixe: "LCKC".into(),
///     langue: "EN".into(),
///     // Une ligne : l'illustration A.
///     lignes: vec![ligne(1, Some("BlueEyesWhiteDragon-LCKC-EN-UR-1E.png"))],
///     ailleurs: Default::default(),
/// };
/// let scans = [
///     scan("BlueEyesWhiteDragon-LCKC-EN-UR-1E.png", ""),
///     scan("BlueEyesWhiteDragon-LCKC-EN-UR-1E-B.png", "B"),
/// ];
/// let t = tableau(&carte, &scans);
/// assert!(t.connu);
/// assert_eq!(t.libelles_completes(), ["Scan"], "A est dans le classeur");
/// assert_eq!(t.colonnes.len(), 1);
/// assert_eq!(t.colonnes[0].libelle(), "Scan B");
///
/// // B porté par un autre numéro du classeur : il est à lui.
/// carte.ailleurs.insert(("blueeyeswhitedragon".into(), "UR".into(), "B".into()));
/// let t = tableau(&carte, &scans);
/// assert!(t.colonnes.is_empty());
/// assert_eq!(t.ecartes, 1);
/// ```
#[must_use]
pub fn tableau(carte: &Carte, scans: &[Scan]) -> Tableau {
    // L'abréviation Yugipedia de chaque rareté du classeur.
    let raretes: Vec<(String, String)> = carte
        .raretes()
        .into_iter()
        .map(|r| {
            let abbr = artwork::abbr_rarete(&r).to_ascii_uppercase();
            (r, abbr)
        })
        .filter(|(_, abbr)| !abbr.is_empty())
        .collect();

    // Ce que les lignes portent : (rareté, suffixe), pour la rareté de la
    // ligne seulement — une image d'emprunt ne couvre rien.
    let mut en_place: BTreeSet<(String, String)> = BTreeSet::new();
    for l in &carte.lignes {
        let Some(fichier) = &l.scan else { continue };
        let Some((_, infos)) = artwork::decouper_fichier(fichier, &carte.prefixe) else {
            continue;
        };
        let abbr = artwork::abbr_rarete(&l.rarete).to_ascii_uppercase();
        if !abbr.is_empty() && infos.rarete_abbr.eq_ignore_ascii_case(&abbr) {
            en_place.insert((abbr, artwork::suffixe(fichier, &carte.prefixe)));
        }
    }

    // L'édition voulue pour chaque rareté : celle de sa première ligne qui
    // en déclare une.
    let edition_de = |rarete: &str| {
        carte
            .lignes
            .iter()
            .filter(|l| l.rarete == rarete)
            .find_map(|l| edition_fichier(l.edition.as_deref()))
    };

    // suffixe → rareté du classeur → meilleur fichier.
    let mut colonnes: BTreeMap<(u8, u32, String), BTreeMap<usize, &Scan>> = BTreeMap::new();
    let mut ecartes = 0;
    for scan in scans {
        let Some((i, (rarete, _))) = raretes
            .iter()
            .enumerate()
            .find(|(_, (_, abbr))| *abbr == scan.rarete)
        else {
            continue;
        };
        let illustration = (scan.rarete.clone(), scan.suffixe.clone());
        if !en_place.contains(&illustration)
            && carte.ailleurs.contains(&(
                scan.cle.clone(),
                scan.rarete.clone(),
                scan.suffixe.clone(),
            ))
        {
            ecartes += 1;
            continue;
        }
        let voulue = edition_de(rarete.as_str());
        let cellules = colonnes.entry(ordre_colonne(&scan.suffixe)).or_default();
        let meilleur = cellules.get(&i).is_none_or(|actuel| {
            (rang_edition(&scan.edition, voulue), &scan.fichier)
                < (rang_edition(&actuel.edition, voulue), &actuel.fichier)
        });
        if meilleur {
            cellules.insert(i, scan);
        }
    }

    let mut tableau = Tableau {
        connu: !scans.is_empty(),
        ecartes,
        ..Tableau::default()
    };
    for (rang, ((_, _, suffixe), cellules)) in colonnes.into_iter().enumerate() {
        let par_rarete: Vec<Anomalie> = cellules
            .into_iter()
            .filter_map(|(i, scan)| {
                let (rarete, abbr) = raretes.get(i)?;
                Some(Anomalie {
                    id: None,
                    nom: carte.nom.clone(),
                    art_a_image_uuid: String::new(),
                    art_b_image_uuid: format!("{PREFIXE_SCAN}{}", scan.fichier),
                    art_index: u32::try_from(rang + 1).unwrap_or(1),
                    set_code_prefix: carte.classeur.clone(),
                    missing_set_code: carte.set_code.clone(),
                    missing_set_rarity: rarete.clone(),
                    image_url: scan.url.clone(),
                    image_url_small: scan.url.clone(),
                    image_id: Some(artwork::id_synthetique(&scan.fichier)),
                    corrige: en_place.contains(&(abbr.clone(), suffixe.clone())),
                })
            })
            .collect();
        let colonne = ArtworkPropose {
            art_index: u32::try_from(rang + 1).unwrap_or(1),
            image_uuid: format!("{PREFIXE_COLONNE}{suffixe}"),
            par_rarete,
            origine: Origine::Yugipedia { suffixe },
        };
        if colonne.par_rarete.iter().all(|a| a.corrige) {
            tableau.completes.push(colonne);
        } else {
            tableau.colonnes.push(colonne);
        }
    }
    tableau
}

/// Le fichier d'un scan proposé, d'après son uuid.
fn fichier_du_scan(anomalie: &Anomalie) -> Option<&str> {
    anomalie.art_b_image_uuid.strip_prefix(PREFIXE_SCAN)
}

/// Ajoute au classeur la ligne d'un scan — ou, pour toute autre
/// proposition, la correction ordinaire ([`anomalies::corriger`]).
///
/// La ligne hérite de tout sauf de l'image, comme toute correction ; le
/// scan y ajoute deux choses que l'ordinaire ignore :
///
/// - **déjà là** se juge sur le **fichier**, pas seulement sur l'identifiant :
///   une ligne dont `images_tirage` a posé le scan garde l'identifiant
///   YGOPRODeck, et la reprendre ferait un doublon ;
/// - la vignette est le scan lui-même, et un scan `EA` naît Overframe.
///
/// # Errors
///
/// Rend une erreur si le classeur ne peut pas être lu ou écrit.
pub fn ajouter(paths: &Paths, anomalie: &Anomalie) -> Result<Correction> {
    let Some(fichier) = fichier_du_scan(anomalie) else {
        return anomalies::corriger(paths, anomalie);
    };
    let chemin = paths.classeur_db(&anomalie.set_code_prefix);
    if !chemin.is_file() {
        return Ok(Correction::ClasseurAbsent);
    }
    {
        let conn = ygo_db::connexion::ouvrir_lecture_seule(&chemin)?;
        let mut requete = conn.prepare(
            "SELECT COALESCE(card_image_url,'') FROM cards \
              WHERE name = ?1 AND set_code = ?2 AND rarity = ?3",
        )?;
        let urls = requete
            .query_map(
                (
                    &anomalie.nom,
                    &anomalie.missing_set_code,
                    &anomalie.missing_set_rarity,
                ),
                |l| l.get::<_, String>(0),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if urls.iter().any(|u| scan_de(u).as_deref() == Some(fichier)) {
            return Ok(Correction::DejaPresente);
        }
    }
    let correction = anomalies::corriger(paths, anomalie)?;
    if let Correction::Ajoutee { rowid, .. } = &correction {
        let conn = ygo_db::connexion::ouvrir(&chemin)?;
        conn.execute(
            "UPDATE cards SET card_image_small = ?1 WHERE rowid = ?2",
            (&anomalie.image_url_small, rowid),
        )?;
        let prefixe = artwork::prefixe_set(&anomalie.missing_set_code);
        if artwork::suffixe(fichier, &prefixe).starts_with("EA")
            && colonne_existe(&conn, "extended_art")?
        {
            conn.execute(
                "UPDATE cards SET extended_art = 1 WHERE rowid = ?1",
                [rowid],
            )?;
        }
    }
    Ok(correction)
}

/// L'image qu'une ligne portait avant « Mettre sur cette carte » — de quoi
/// revenir en arrière.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageAvant {
    /// Le classeur.
    pub classeur: String,
    /// La ligne.
    pub rowid: i64,
    /// `card_image_uuid`.
    pub uuid: Option<String>,
    /// `card_image_id`.
    pub image_id: Option<i64>,
    /// `card_image_url`.
    pub url: Option<String>,
    /// `card_image_small`.
    pub small: Option<String>,
}

/// Pose un scan **sur une ligne existante**, au lieu d'en ajouter une.
///
/// C'est le geste qui manquait pour *modifier* l'artwork d'une carte : une
/// ligne qui montre l'image YGOPRODeck de l'illustration prend le scan de
/// **son** tirage. Possession, quantité, état, édition : rien d'autre ne
/// change.
///
/// La ligne doit être du numéro et de la rareté du scan ; sinon rien n'est
/// écrit et la fonction rend `None`. L'uuid `yugipedia:` protège ensuite le
/// choix : la passe artworks ne retouche pas une ligne servie, et
/// `images_tirage` laisse les identifiants négatifs.
///
/// # Errors
///
/// Rend une erreur si le classeur ne peut pas être écrit.
pub fn mettre_sur_la_carte(
    paths: &Paths,
    rowid: i64,
    anomalie: &Anomalie,
) -> Result<Option<ImageAvant>> {
    let chemin = paths.classeur_db(&anomalie.set_code_prefix);
    if !chemin.is_file() {
        return Ok(None);
    }
    let conn = ygo_db::connexion::ouvrir(&chemin)?;
    let avant = conn
        .query_row(
            "SELECT card_image_uuid, card_image_id, card_image_url, card_image_small \
               FROM cards WHERE rowid = ?1 AND set_code = ?2 AND rarity = ?3",
            (
                rowid,
                &anomalie.missing_set_code,
                &anomalie.missing_set_rarity,
            ),
            |l| {
                Ok(ImageAvant {
                    classeur: anomalie.set_code_prefix.clone(),
                    rowid,
                    uuid: l.get(0)?,
                    image_id: l.get(1)?,
                    url: l.get(2)?,
                    small: l.get(3)?,
                })
            },
        )
        .optional()?;
    let Some(avant) = avant else {
        return Ok(None);
    };
    conn.execute(
        "UPDATE cards SET card_image_uuid = ?1, card_image_id = ?2, \
                card_image_url = ?3, card_image_small = ?4 \
          WHERE rowid = ?5",
        rusqlite::params![
            anomalie.art_b_image_uuid,
            anomalie.image_id,
            anomalie.image_url,
            anomalie.image_url_small,
            rowid
        ],
    )?;
    Ok(Some(avant))
}

/// Rend à une ligne l'image qu'elle avait avant [`mettre_sur_la_carte`].
///
/// # Errors
///
/// Rend une erreur si le classeur ne peut pas être écrit.
pub fn retablir(paths: &Paths, avant: &ImageAvant) -> Result<bool> {
    let chemin = paths.classeur_db(&avant.classeur);
    if !chemin.is_file() {
        return Ok(false);
    }
    let conn = ygo_db::connexion::ouvrir(&chemin)?;
    let n = conn.execute(
        "UPDATE cards SET card_image_uuid = ?1, card_image_id = ?2, \
                card_image_url = ?3, card_image_small = ?4 \
          WHERE rowid = ?5",
        rusqlite::params![
            avant.uuid,
            avant.image_id,
            avant.url,
            avant.small,
            avant.rowid
        ],
    )?;
    Ok(n > 0)
}

/// Retire une ligne qu'un ajout vient de créer — tant qu'elle n'a aucun
/// exemplaire.
///
/// L'écran ne l'offre que pour les lignes qu'il a lui-même ajoutées dans la
/// séance : il connaît leur `rowid`, et ne touche jamais à une ligne qui
/// était là avant lui.
///
/// # Errors
///
/// Rend une erreur si le classeur ne peut pas être écrit.
pub fn retirer_ajout(paths: &Paths, classeur: &str, rowid: i64) -> Result<bool> {
    let chemin = paths.classeur_db(classeur);
    if !chemin.is_file() {
        return Ok(false);
    }
    let conn = ygo_db::connexion::ouvrir(&chemin)?;
    let n = conn.execute(
        "DELETE FROM cards WHERE rowid = ?1 AND COALESCE(quantite,0) = 0",
        [rowid],
    )?;
    Ok(n > 0)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    const UR: &str = "Ultra Rare";
    const SCR: &str = "Secret Rare";

    fn url(f: &str) -> String {
        artwork::url_fichier(f)
    }

    /// Une installation : un classeur `LCKC`, et ce qu'on lui met.
    fn installation() -> (tempfile::TempDir, Paths) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(tmp.path());
        paths.creer_dossiers().unwrap();
        std::fs::create_dir_all(paths.img_small()).unwrap();
        std::fs::create_dir_all(paths.dossier_classeur("LCKC")).unwrap();
        let conn = rusqlite::Connection::open(paths.classeur_db("LCKC")).unwrap();
        conn.execute_batch(ygo_db::schema::DDL_CLASSEUR).unwrap();
        (tmp, paths)
    }

    /// Une ligne, d'adresse et d'identifiant donnés.
    fn inserer(paths: &Paths, code: &str, rarete: &str, url: &str, id: i64, quantite: i64) -> i64 {
        let conn = rusqlite::Connection::open(paths.classeur_db("LCKC")).unwrap();
        conn.execute(
            "INSERT INTO cards (card_uuid, card_image_uuid, set_code, rarity, rarity_code,
                                set_name, name, name_fr, card_image_url, card_image_small,
                                card_image_id, sort_order, card_type, atk, def_val, level,
                                attribute, race, possessed, quantite, edition)
             VALUES ('', '', ?1, ?2, '', 'Legendary Collection Kaiba', 'Blue-Eyes White Dragon',
                     'Dragon Blanc aux Yeux Bleus', ?3, ?3, ?4, 7, 'Normal Monster', 3000, 2500,
                     8, 'LIGHT', 'Dragon', ?5, ?5, NULL)",
            rusqlite::params![code, rarete, url, id, quantite],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn deposer(paths: &Paths, fichier: &str) {
        std::fs::write(paths.img_small().join(fichier), b"png").unwrap();
    }

    /// Le cas de l'utilisateur : la carte montre l'image YGOPRODeck, les
    /// scans du tirage sont déjà sur le disque — l'écran les trouve sans
    /// réseau, et seulement eux.
    #[test]
    fn les_scans_du_disque_sont_trouves_sans_reseau() {
        let (_t, paths) = installation();
        inserer(
            &paths,
            "LCKC-EN001",
            UR,
            "https://images.ygoprodeck.com/images/cards/89631139.jpg",
            89_631_139,
            1,
        );
        for f in [
            "BlueEyesWhiteDragon-LCKC-EN-UR-1E.png",
            "BlueEyesWhiteDragon-LCKC-EN-UR-1E-B.png",
            // Une autre carte, un autre set, une autre langue, un contexte :
            // rien de tout cela n'est à elle.
            "BlueEyesUltimateDragon-LCKC-EN-ScR-UE.png",
            "BlueEyesWhiteDragon-LDK2-EN-C-1E-2.png",
            "BlueEyesWhiteDragon-LCKC-FR-UR-1E.png",
            "BlueEyesWhiteDragon-LCKC-EN-OP.png",
            "89631139.jpg",
        ] {
            deposer(&paths, f);
        }
        let carte = lire_carte(&paths, "LCKC", "lckc-en001").unwrap().unwrap();
        assert_eq!(carte.set_code, "LCKC-EN001");
        assert_eq!(carte.langue, "EN");
        assert_eq!(carte.lignes[0].scan, None, "une image YGOPRODeck");

        let scans = scans_locaux(&paths, &carte);
        let noms: Vec<&str> = scans.iter().map(|s| s.fichier.as_str()).collect();
        assert_eq!(
            noms,
            [
                "BlueEyesWhiteDragon-LCKC-EN-UR-1E-B.png",
                "BlueEyesWhiteDragon-LCKC-EN-UR-1E.png"
            ]
        );
        // L'adresse se déduit du nom.
        assert_eq!(scans[1].url, url("BlueEyesWhiteDragon-LCKC-EN-UR-1E.png"));

        // Rien n'est en place : les deux illustrations sont à prendre.
        let t = tableau(&carte, &scans);
        assert!(t.connu);
        assert!(t.completes.is_empty());
        let libelles: Vec<String> = t.colonnes.iter().map(ArtworkPropose::libelle).collect();
        assert_eq!(libelles, ["Scan", "Scan B"], "le tirage nu d'abord");
        assert!(t.colonnes.iter().all(|c| c.par_rarete.len() == 1));
    }

    /// `cardinfo.db` connaît le scan du tirage : c'est une source locale,
    /// avant le disque — et une révision archivée y est rendue à son fichier.
    #[test]
    fn set_prints_est_une_source_locale() {
        let (_t, paths) = installation();
        inserer(&paths, "LCKC-EN001", UR, "", 89_631_139, 0);
        let cardinfo = rusqlite::Connection::open(paths.cardinfo_db()).unwrap();
        cardinfo
            .execute_batch(
                "CREATE TABLE set_prints (set_code TEXT, print_image_url TEXT);
                 INSERT INTO set_prints VALUES ('LCKC-EN001',
                   'https://ms.yugipedia.com//2/2b/BlueEyesWhiteDragon-LCKC-EN-UR-UE-C.png');
                 INSERT INTO set_prints VALUES ('LCKC-EN001',
                   'https://ms.yugipedia.com//archive/1/11/20260101084736%21BlueEyesWhiteDragon-LCKC-EN-UR-UE.png');
                 INSERT INTO set_prints VALUES ('LCKC-EN002',
                   'https://ms.yugipedia.com//x/xy/AutreCarte-LCKC-EN-UR-UE.png');",
            )
            .unwrap();
        drop(cardinfo);
        let carte = lire_carte(&paths, "LCKC", "LCKC-EN001").unwrap().unwrap();
        let scans = scans_locaux(&paths, &carte);
        let par_nom: BTreeMap<&str, &str> = scans
            .iter()
            .map(|s| (s.fichier.as_str(), s.url.as_str()))
            .collect();
        assert_eq!(par_nom.len(), 2, "{par_nom:?}");
        assert_eq!(
            par_nom["BlueEyesWhiteDragon-LCKC-EN-UR-UE-C.png"],
            "https://ms.yugipedia.com//2/2b/BlueEyesWhiteDragon-LCKC-EN-UR-UE-C.png",
            "l'adresse de la base est gardée"
        );
        assert_eq!(
            par_nom["BlueEyesWhiteDragon-LCKC-EN-UR-UE.png"],
            url("BlueEyesWhiteDragon-LCKC-EN-UR-UE.png"),
            "la révision archivée cède la place au fichier courant"
        );
    }

    /// Une illustration en place l'est quelle que soit l'édition du scan ;
    /// une colonne entièrement en place ne se propose plus, mais se compte.
    #[test]
    fn l_illustration_en_place_l_est_dans_toutes_les_editions() {
        let (_t, paths) = installation();
        inserer(
            &paths,
            "LCKC-EN001",
            UR,
            &url("BlueEyesWhiteDragon-LCKC-EN-UR-UE-C.png"),
            89_631_139,
            0,
        );
        let carte = lire_carte(&paths, "LCKC", "LCKC-EN001").unwrap().unwrap();
        let scan = |f: &str| Scan {
            fichier: f.to_owned(),
            url: url(f),
            rarete: "UR".to_owned(),
            edition: artwork::decouper_fichier(f, "LCKC").unwrap().1.edition,
            suffixe: artwork::suffixe(f, "LCKC"),
            cle: "blueeyeswhitedragon".to_owned(),
        };
        let t = tableau(
            &carte,
            &[
                scan("BlueEyesWhiteDragon-LCKC-EN-UR-1E.png"),
                scan("BlueEyesWhiteDragon-LCKC-EN-UR-1E-B.png"),
                scan("BlueEyesWhiteDragon-LCKC-EN-UR-1E-C.png"),
                scan("BlueEyesWhiteDragon-LCKC-EN-UR-1E-D.png"),
            ],
        );
        assert_eq!(
            t.libelles_completes(),
            ["Scan C"],
            "l'UE en place couvre la 1E"
        );
        let libelles: Vec<String> = t.colonnes.iter().map(ArtworkPropose::libelle).collect();
        assert_eq!(libelles, ["Scan", "Scan B", "Scan D"]);
    }

    /// Chaque rareté a son fichier, et la meilleure édition gagne : celle de
    /// la ligne, puis la première.
    #[test]
    fn chaque_rarete_a_son_fichier_et_sa_meilleure_edition() {
        let (_t, paths) = installation();
        inserer(&paths, "LCKC-EN001", UR, "", 1, 0);
        inserer(&paths, "LCKC-EN001", SCR, "", 1, 0);
        let conn = rusqlite::Connection::open(paths.classeur_db("LCKC")).unwrap();
        conn.execute(
            "UPDATE cards SET edition = 'unlimited' WHERE rarity = ?1",
            [SCR],
        )
        .unwrap();
        drop(conn);
        let carte = lire_carte(&paths, "LCKC", "LCKC-EN001").unwrap().unwrap();
        let scan = |f: &str, rarete: &str, edition: &str| Scan {
            fichier: f.to_owned(),
            url: url(f),
            rarete: rarete.to_owned(),
            edition: edition.to_owned(),
            suffixe: String::new(),
            cle: "x".to_owned(),
        };
        let t = tableau(
            &carte,
            &[
                scan("X-LCKC-EN-UR-UE.png", "UR", "UE"),
                scan("X-LCKC-EN-UR-1E.png", "UR", "1E"),
                scan("X-LCKC-EN-ScR-1E.png", "SCR", "1E"),
                scan("X-LCKC-EN-ScR-UE.png", "SCR", "UE"),
                // Une rareté que le classeur n'a pas pour ce numéro.
                scan("X-LCKC-EN-C-1E.png", "C", "1E"),
            ],
        );
        assert_eq!(t.colonnes.len(), 1);
        let c = &t.colonnes[0];
        assert_eq!(c.par_rarete.len(), 2, "ni la Common, ni deux fois l'UR");
        assert_eq!(c.pour(UR).unwrap().image_url, url("X-LCKC-EN-UR-1E.png"));
        assert_eq!(
            c.pour(SCR).unwrap().image_url,
            url("X-LCKC-EN-ScR-UE.png"),
            "l'édition de la ligne passe devant la première"
        );
        let a = c.pour(UR).unwrap();
        assert_eq!(a.art_b_image_uuid, "yugipedia:X-LCKC-EN-UR-1E.png");
        assert_eq!(
            a.image_id,
            Some(artwork::id_synthetique("X-LCKC-EN-UR-1E.png"))
        );
        assert_eq!(a.set_code_prefix, "LCKC");
        assert!(!a.corrige);
    }

    /// Sans aucun scan, Yugipedia ne sait rien : l'écran rendra la main à
    /// YGOPRODeck.
    #[test]
    fn sans_scan_yugipedia_ne_sait_rien() {
        let (_t, paths) = installation();
        inserer(&paths, "LCKC-EN001", UR, "", 1, 0);
        let carte = lire_carte(&paths, "LCKC", "LCKC-EN001").unwrap().unwrap();
        let t = tableau(&carte, &scans_locaux(&paths, &carte));
        assert!(!t.connu);
        assert!(t.colonnes.is_empty() && t.completes.is_empty());
        assert!(lire_carte(&paths, "LCKC", "LCKC-EN999").unwrap().is_none());
        assert!(lire_carte(&paths, "ABSENT", "X-EN001").unwrap().is_none());
    }

    /// Les colonnes se rangent comme on lit : le tirage nu, B, C, puis les
    /// variantes, l'extended art en dernier.
    #[test]
    fn l_ordre_des_colonnes() {
        let mut suffixes = vec!["EA", "AA2", "C", "", "AA", "J", "2", "B", "KAIBA"];
        suffixes.sort_by_key(|s| ordre_colonne(s));
        assert_eq!(
            suffixes,
            ["", "2", "B", "C", "J", "AA", "AA2", "EA", "KAIBA"]
        );
    }

    /// Ajouter un scan : une ligne non possédée, qui porte le scan, sa
    /// vignette et son identifiant ; un second ajout est reconnu.
    #[test]
    fn ajouter_un_scan_cree_la_ligne_et_ne_la_double_pas() {
        let (_t, paths) = installation();
        inserer(
            &paths,
            "LCKC-EN001",
            UR,
            "https://images.ygoprodeck.com/images/cards/1.jpg",
            1,
            2,
        );
        let fichier = "BlueEyesWhiteDragon-LCKC-EN-UR-1E-B.png";
        deposer(&paths, fichier);
        let carte = lire_carte(&paths, "LCKC", "LCKC-EN001").unwrap().unwrap();
        let t = tableau(&carte, &scans_locaux(&paths, &carte));
        let proposition = t.colonnes[0].pour(UR).unwrap().clone();

        let rowid = match ajouter(&paths, &proposition).unwrap() {
            Correction::Ajoutee { rowid, .. } => rowid,
            autre => panic!("{autre:?}"),
        };
        let conn = rusqlite::Connection::open(paths.classeur_db("LCKC")).unwrap();
        let (uuid, id, url_ligne, small, quantite, ext): (String, i64, String, String, i64, i64) =
            conn.query_row(
                "SELECT card_image_uuid, card_image_id, card_image_url, card_image_small,
                        quantite, extended_art FROM cards WHERE rowid = ?1",
                [rowid],
                |l| {
                    Ok((
                        l.get(0)?,
                        l.get(1)?,
                        l.get(2)?,
                        l.get(3)?,
                        l.get(4)?,
                        l.get(5)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(uuid, format!("yugipedia:{fichier}"));
        assert_eq!(id, artwork::id_synthetique(fichier));
        assert_eq!(url_ligne, url(fichier));
        assert_eq!(
            small,
            url(fichier),
            "la vignette est le scan, pas celle du témoin"
        );
        assert_eq!(quantite, 0, "née non possédée");
        assert_eq!(ext, 0);
        drop(conn);

        assert_eq!(
            ajouter(&paths, &proposition).unwrap(),
            Correction::DejaPresente
        );

        // Relue, la carte a l'illustration B en place.
        let carte = lire_carte(&paths, "LCKC", "LCKC-EN001").unwrap().unwrap();
        let t = tableau(&carte, &scans_locaux(&paths, &carte));
        assert_eq!(t.libelles_completes(), ["Scan B"]);

        // Elle se retire, tant qu'elle n'a pas d'exemplaire.
        assert!(retirer_ajout(&paths, "LCKC", rowid).unwrap());
        assert!(
            !retirer_ajout(&paths, "LCKC", rowid).unwrap(),
            "déjà partie"
        );
    }

    /// Le scan qu'`images_tirage` a posé garde l'identifiant YGOPRODeck : il
    /// est déjà là, et l'ajout le reconnaît à son fichier.
    #[test]
    fn un_scan_pose_sous_un_identifiant_ygoprodeck_est_deja_la() {
        let (_t, paths) = installation();
        let fichier = "BlueEyesWhiteDragon-LCKC-EN-UR-1E.png";
        inserer(&paths, "LCKC-EN001", UR, &url(fichier), 89_631_139, 0);
        let anomalie = Anomalie {
            id: None,
            nom: "Blue-Eyes White Dragon".into(),
            art_a_image_uuid: String::new(),
            art_b_image_uuid: format!("yugipedia:{fichier}"),
            art_index: 1,
            set_code_prefix: "LCKC".into(),
            missing_set_code: "LCKC-EN001".into(),
            missing_set_rarity: UR.into(),
            image_url: url(fichier),
            image_url_small: url(fichier),
            image_id: Some(artwork::id_synthetique(fichier)),
            corrige: false,
        };
        assert_eq!(
            ajouter(&paths, &anomalie).unwrap(),
            Correction::DejaPresente
        );
    }

    /// Un scan `EA` naît Overframe.
    #[test]
    fn un_scan_extended_art_nait_overframe() {
        let (_t, paths) = installation();
        inserer(&paths, "LCKC-EN001", UR, "", 1, 0);
        let fichier = "BlueEyesWhiteDragon-LCKC-EN-UR-1E-EA.png";
        deposer(&paths, fichier);
        let carte = lire_carte(&paths, "LCKC", "LCKC-EN001").unwrap().unwrap();
        let t = tableau(&carte, &scans_locaux(&paths, &carte));
        let rowid = match ajouter(&paths, t.colonnes[0].pour(UR).unwrap()).unwrap() {
            Correction::Ajoutee { rowid, .. } => rowid,
            autre => panic!("{autre:?}"),
        };
        let conn = rusqlite::Connection::open(paths.classeur_db("LCKC")).unwrap();
        let ext: i64 = conn
            .query_row(
                "SELECT extended_art FROM cards WHERE rowid = ?1",
                [rowid],
                |l| l.get(0),
            )
            .unwrap();
        assert_eq!(ext, 1);
    }

    /// « Mettre sur cette carte » change l'image de la ligne et rien
    /// d'autre ; « Annuler » la rend telle qu'elle était.
    #[test]
    fn mettre_sur_la_carte_puis_annuler() {
        let (_t, paths) = installation();
        let avant_url = "https://images.ygoprodeck.com/images/cards/89631139.jpg";
        let rowid = inserer(&paths, "LCKC-EN001", UR, avant_url, 89_631_139, 3);
        let fichier = "BlueEyesWhiteDragon-LCKC-EN-UR-1E.png";
        deposer(&paths, fichier);
        let carte = lire_carte(&paths, "LCKC", "LCKC-EN001").unwrap().unwrap();
        let t = tableau(&carte, &scans_locaux(&paths, &carte));
        let scan = t.colonnes[0].pour(UR).unwrap().clone();

        let avant = mettre_sur_la_carte(&paths, rowid, &scan).unwrap().unwrap();
        assert_eq!(avant.url.as_deref(), Some(avant_url));
        assert_eq!(avant.image_id, Some(89_631_139));

        let lire = || {
            let conn = rusqlite::Connection::open(paths.classeur_db("LCKC")).unwrap();
            conn.query_row(
                "SELECT card_image_url, card_image_id, card_image_uuid, quantite
                   FROM cards WHERE rowid = ?1",
                [rowid],
                |l| {
                    Ok((
                        l.get::<_, String>(0)?,
                        l.get::<_, i64>(1)?,
                        l.get::<_, String>(2)?,
                        l.get::<_, i64>(3)?,
                    ))
                },
            )
            .unwrap()
        };
        let (u, id, uuid, q) = lire();
        assert_eq!(u, url(fichier));
        assert_eq!(id, artwork::id_synthetique(fichier));
        assert_eq!(uuid, format!("yugipedia:{fichier}"));
        assert_eq!(q, 3, "les exemplaires restent");

        // L'illustration est désormais en place.
        let carte = lire_carte(&paths, "LCKC", "LCKC-EN001").unwrap().unwrap();
        assert_eq!(
            tableau(&carte, &scans_locaux(&paths, &carte)).libelles_completes(),
            ["Scan"]
        );

        assert!(retablir(&paths, &avant).unwrap());
        let (u, id, _, q) = lire();
        assert_eq!((u.as_str(), id, q), (avant_url, 89_631_139, 3));

        // Une ligne d'une autre rareté ne prend pas ce scan.
        let autre = inserer(&paths, "LCKC-EN001", SCR, "", 1, 0);
        assert_eq!(mettre_sur_la_carte(&paths, autre, &scan).unwrap(), None);
    }

    /// Une ligne possédée ne se retire pas.
    #[test]
    fn une_ligne_possedee_ne_se_retire_pas() {
        let (_t, paths) = installation();
        let rowid = inserer(&paths, "LCKC-EN001", UR, "", 1, 1);
        assert!(!retirer_ajout(&paths, "LCKC", rowid).unwrap());
    }

    /// Un nom de fichier ne dit pas son numéro : `RA04` imprime Dark Magician
    /// sous deux numéros, et `…-QCScR-1E-AA2.png` sert les deux noms. Ce
    /// qu'un autre numéro porte est à lui ; ce que personne ne porte reste
    /// proposé.
    #[test]
    fn un_scan_porte_par_un_autre_numero_lui_appartient() {
        let (_t, paths) = installation();
        let f = |s: &str| format!("BlueEyesWhiteDragon-LCKC-EN-UR-1E{s}.png");
        inserer(&paths, "LCKC-EN001", UR, &url(&f("-AA")), 1, 0);
        for s in ["", "-AA", "-AA2"] {
            inserer(&paths, "LCKC-EN106", UR, &url(&f(s)), 1, 0);
        }
        for s in ["", "-AA", "-AA2", "-AA3"] {
            deposer(&paths, &f(s));
        }
        let carte = lire_carte(&paths, "LCKC", "LCKC-EN001").unwrap().unwrap();
        assert_eq!(carte.ailleurs.len(), 3);
        let t = tableau(&carte, &scans_locaux(&paths, &carte));
        assert_eq!(
            t.libelles_completes(),
            ["Scan AA"],
            "porté ici et là-bas : en place"
        );
        assert_eq!(t.ecartes, 2, "le nu et AA2 sont à EN106");
        let libelles: Vec<String> = t.colonnes.iter().map(ArtworkPropose::libelle).collect();
        assert_eq!(libelles, ["Scan AA3"], "personne ne le porte");

        // Vu de l'autre numéro, rien n'est écarté.
        let autre = lire_carte(&paths, "LCKC", "LCKC-EN106").unwrap().unwrap();
        let t = tableau(&autre, &scans_locaux(&paths, &autre));
        assert_eq!(t.ecartes, 0);
        assert_eq!(t.libelles_completes(), ["Scan", "Scan AA", "Scan AA2"]);
    }

    /// La récolte de Yugipedia complète celle du disque sans la remplacer.
    #[test]
    fn la_fusion_garde_le_local_et_ajoute_le_distant() {
        let scan = |f: &str, u: &str| Scan {
            fichier: f.to_owned(),
            url: u.to_owned(),
            rarete: "UR".to_owned(),
            edition: "1E".to_owned(),
            suffixe: String::new(),
            cle: "x".to_owned(),
        };
        let tous = fusionner(
            vec![scan("a.png", "local")],
            &[scan("a.png", "distant"), scan("b.png", "distant")],
        );
        assert_eq!(tous.len(), 2);
        assert_eq!(tous[0].url, "local");
        assert_eq!(tous[1].fichier, "b.png");
    }
}
