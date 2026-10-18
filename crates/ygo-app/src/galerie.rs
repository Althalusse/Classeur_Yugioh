// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Chaque scan à son numéro — le classeur réaligné sur la galerie du set.
//!
//! # Pourquoi — 2026-10-18
//!
//! Retour d'usage : « l'artwork actuel du `RA04-FR002` n'est pas cohérent
//! avec l'artwork IRL, qui correspondrait à un autre, `RA04-FR108` ». Un nom
//! de fichier Yugipedia ne dit pas son numéro, et `RA04` imprime Blue-Eyes
//! White Dragon et Dark Magician sous deux numéros chacun : la passe
//! artworks et `images_tirage` répartissaient les fichiers d'une carte sans
//! savoir lequel allait où. État relevé dans « Projet Clean » :
//!
//! | numéro | rareté | la galerie | le classeur |
//! |---|---|---|---|
//! | `EN002` | QCScR | nu | `AA` |
//! | `EN108` | QCScR | `AA`, `AA2` | nu, `AA` |
//! | `EN001` | QCScR | nu | `AA` |
//! | `EN106` | PlScR | nu, `AA`…`AA4` | `AA`…`AA4`, et `AA2` une 2e fois |
//! | `EN106` | QCScR | `AA`…`AA5` | `AA`…`AA4`, et le nu d'`EN001` |
//!
//! La galerie ([`ygo_sources::yugipedia::galerie`]) donne, pour chaque numéro
//! et chaque rareté, les illustrations qu'il porte. Ce module y ramène les
//! lignes qui la contredisent.
//!
//! # La règle — [`planifier`]
//!
//! Pour chaque `(numéro, rareté)` que la galerie montre, ses illustrations
//! attendues — les suffixes de fichier, dans l'ordre de la page — sont
//! réparties entre ses lignes :
//!
//! 1. une ligne dont l'utilisateur a **choisi** l'image (« Mettre sur cette
//!    carte ») la garde, quoi qu'en dise la galerie ;
//! 2. une ligne qui montre **déjà** l'une des illustrations attendues la garde
//!    — les lignes possédées d'abord, puis celles qu'une passe a servies :
//!    quand deux lignes montrent la même, c'est l'autre qui change (`EN106`
//!    : la ligne aux quatre exemplaires garde son `AA2`) ;
//! 3. les autres reçoivent ce qui reste, cadre normal d'abord, un `EA` sur
//!    une Overframe seulement — si elles montrent le scan d'une **autre**
//!    illustration, si elles n'ont pas d'image, ou si l'illustration attendue
//!    est une illustration alternative, qu'une image YGOPRODeck ne peut pas
//!    montrer. Une image YGOPRODeck face au fichier nu reste : c'est
//!    l'illustration de la carte.
//!
//! Une ligne en trop — plus de lignes que d'illustrations — reste comme elle
//! est. Une illustration en trop attend la ligne que la Set list fera créer.
//!
//! # Le fichier — [`fichier_pour`]
//!
//! Parmi les scans connus de la carte, celui de la rareté et du suffixe, à
//! l'édition de la ligne, sinon `1E`, aucune, `UE`, `LE` ; jamais un fichier
//! à jeton de contexte. Connus d'abord sans réseau — les lignes du classeur,
//! `img/small` —, puis par une requête `allimages` par carte, gardée 30 jours.
//!
//! # Les choix de l'utilisateur
//!
//! Notés dans la table `meta` du classeur, sous `choix_artwork:<rowid>`
//! ([`noter_choix`]) : par « Mettre sur cette carte » et par l'ajout d'un
//! scan. Une ligne ainsi choisie n'est jamais réalignée.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use rusqlite::Connection;
use ygo_core::paths::Paths;
use ygo_sources::yugipedia::artwork;
use ygo_sources::yugipedia::galerie::{EntreeGalerie, Galerie};

use crate::artworks::colonne_existe;
use crate::error::Result;
use crate::scans::{scan_de, PREFIXE_SCAN};

/// Le préfixe des clés `meta` qui notent un choix d'image de l'utilisateur.
pub const CLE_CHOIX: &str = "choix_artwork:";

/// Une ligne du classeur, réduite à ce que le réalignement regarde.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Ligne {
    /// `rowid`.
    pub rowid: i64,
    /// Nom anglais de la carte.
    pub name: String,
    /// Numéro, en majuscules.
    pub set_code: String,
    /// Rareté, telle qu'écrite dans le classeur.
    pub rarity: String,
    /// Tirage Overframe.
    pub overframe: bool,
    /// Édition Scanflip (`1st`, `unlimited`, `limited`), si renseignée.
    pub edition: Option<String>,
    /// Exemplaires possédés.
    pub quantite: i64,
    /// `card_image_url`.
    pub url: String,
    /// `card_image_uuid`.
    pub uuid: String,
}

/// Ce qu'une ligne montre, au regard de son tirage.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Montre {
    /// Aucune adresse d'image.
    Rien,
    /// Une image qui n'est pas un scan Yugipedia — celle d'YGOPRODeck.
    Autre,
    /// Un scan de **cette** carte dans **cette** rareté, de ce suffixe.
    Scan(String),
    /// Le scan Yugipedia d'une autre carte ou d'une autre rareté.
    Etranger,
}

/// Ce que la galerie demande pour une ligne : l'illustration qu'elle doit
/// montrer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reprise {
    /// La ligne.
    pub rowid: i64,
    /// Son numéro.
    pub numero: String,
    /// L'abréviation de sa rareté, en majuscules.
    pub abbr: String,
    /// Le suffixe du fichier attendu (`""`, `AA2`…).
    pub suffixe: String,
    /// Les clés de carte qu'un fichier doit porter.
    pub cles: BTreeSet<String>,
    /// Les noms de carte, tels que Yugipedia les écrit dans un fichier —
    /// pour la requête `allimages`.
    pub segments: BTreeSet<String>,
    /// L'édition Scanflip de la ligne.
    pub edition: Option<String>,
    /// Le scan que la ligne montrait, s'il en était un.
    pub avant: Option<String>,
}

/// Retire de `restants` la première occurrence de `suffixe`.
fn retirer(restants: &mut Vec<String>, suffixe: &str) -> bool {
    match restants
        .iter()
        .position(|s| s.eq_ignore_ascii_case(suffixe))
    {
        Some(i) => {
            restants.remove(i);
            true
        }
        None => false,
    }
}

/// Un nom de carte, tel qu'un nom de fichier l'écrit : lettres et chiffres.
fn compact(nom: &str) -> String {
    nom.chars().filter(char::is_ascii_alphanumeric).collect()
}

fn montre(ligne: &Ligne, cles: &BTreeSet<String>, abbr: &str, prefixe: &str) -> Montre {
    if ligne.url.trim().is_empty() {
        return Montre::Rien;
    }
    let Some(fichier) = scan_de(&ligne.url) else {
        return Montre::Autre;
    };
    match artwork::decouper_fichier(&fichier, prefixe) {
        Some((segment, infos))
            if infos.rarete_abbr.eq_ignore_ascii_case(abbr)
                && cles.contains(&artwork::cle_comparaison(&segment)) =>
        {
            Montre::Scan(artwork::suffixe(&fichier, prefixe))
        }
        _ => Montre::Etranger,
    }
}

/// Le réalignement d'un classeur sur sa galerie — **fonction pure**.
///
/// `prefixe` : le préfixe de set des fichiers (`RA04`) ; `choix` : les lignes
/// dont l'utilisateur a choisi l'image. Cf. l'en-tête du module pour la règle.
#[must_use]
pub fn planifier(
    lignes: &[Ligne],
    galerie: &Galerie,
    prefixe: &str,
    choix: &HashSet<i64>,
) -> Vec<Reprise> {
    let mut groupes: BTreeMap<(String, String), Vec<&Ligne>> = BTreeMap::new();
    for l in lignes {
        let abbr = artwork::abbr_rarete(&l.rarity).to_ascii_uppercase();
        if abbr.is_empty() || !galerie.decrit(&l.set_code, &abbr) {
            continue;
        }
        groupes
            .entry((l.set_code.to_uppercase(), abbr))
            .or_default()
            .push(l);
    }

    let mut reprises = Vec::new();
    for ((numero, abbr), mut du_tirage) in groupes {
        let mut restants = galerie.suffixes(&numero, &abbr);
        let entrees: Vec<&EntreeGalerie> = galerie
            .entrees
            .iter()
            .filter(|e| e.numero.eq_ignore_ascii_case(&numero))
            .collect();
        let mut cles: BTreeSet<String> = entrees.iter().flat_map(|e| e.cles()).collect();
        cles.extend(
            du_tirage
                .iter()
                .map(|l| artwork::cle_comparaison(&l.name))
                .filter(|c| !c.is_empty()),
        );
        let etats: HashMap<i64, Montre> = du_tirage
            .iter()
            .map(|l| (l.rowid, montre(l, &cles, &abbr, prefixe)))
            .collect();
        // Les noms sous lesquels chercher les fichiers : ceux de la page,
        // des lignes, et des scans déjà en place — une carte renommée garde
        // dans ses fichiers le nom d'alors.
        let mut segments: BTreeSet<String> = entrees
            .iter()
            .map(|e| {
                let nom = e.nom.trim();
                let sans = nom
                    .rsplit_once(" (")
                    .filter(|_| nom.ends_with(')'))
                    .map_or(nom, |(avant, _)| avant);
                compact(sans)
            })
            .chain(du_tirage.iter().map(|l| compact(&l.name)))
            .filter(|s| !s.is_empty())
            .collect();
        for l in &du_tirage {
            if let Some((segment, _)) = scan_de(&l.url)
                .as_deref()
                .and_then(|f| artwork::decouper_fichier(f, prefixe))
            {
                if cles.contains(&artwork::cle_comparaison(&segment)) && !segment.is_empty() {
                    segments.insert(segment);
                }
            }
        }

        // 1-2. Ce qui est juste reste : le choix de l'utilisateur, puis les
        // lignes possédées, puis celles qu'une passe a servies.
        let mut faites: HashSet<i64> = HashSet::new();
        let mut par_priorite = du_tirage.clone();
        par_priorite.sort_by_key(|l| {
            (
                Reverse(choix.contains(&l.rowid)),
                Reverse(l.quantite > 0),
                Reverse(l.uuid.starts_with(PREFIXE_SCAN)),
                l.overframe,
                l.rowid,
            )
        });
        for l in &par_priorite {
            let scan = match etats.get(&l.rowid) {
                Some(Montre::Scan(s)) => Some(s.as_str()),
                _ => None,
            };
            if choix.contains(&l.rowid) {
                if let Some(s) = scan {
                    retirer(&mut restants, s);
                }
                faites.insert(l.rowid);
            } else if scan.is_some_and(|s| retirer(&mut restants, s)) {
                faites.insert(l.rowid);
            }
        }

        // 3. Le reste reçoit ce qui reste, cadre normal d'abord.
        du_tirage.sort_by_key(|l| (l.overframe, l.rowid));
        for l in du_tirage {
            if faites.contains(&l.rowid) {
                continue;
            }
            let Some(i) = restants
                .iter()
                .position(|s| s.starts_with("EA") == l.overframe)
            else {
                continue;
            };
            let suffixe = restants.remove(i);
            let etat = etats.get(&l.rowid).cloned().unwrap_or(Montre::Rien);
            // Une image YGOPRODeck face au fichier nu : c'est l'illustration
            // de la carte, `images_tirage` lui donne celle de son tirage.
            if etat == Montre::Autre && suffixe.is_empty() {
                continue;
            }
            reprises.push(Reprise {
                rowid: l.rowid,
                numero: numero.clone(),
                abbr: abbr.clone(),
                suffixe,
                cles: cles.clone(),
                segments: segments.clone(),
                edition: l.edition.clone(),
                avant: match etat {
                    Montre::Scan(_) | Montre::Etranger => scan_de(&l.url),
                    Montre::Rien | Montre::Autre => None,
                },
            });
        }
    }
    reprises
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

/// Le scan d'une reprise parmi des scans connus `(nom, adresse)` : la carte,
/// la rareté, le suffixe et la langue ; l'édition de la ligne, sinon `1E`,
/// aucune, `UE`, `LE` ; jamais un fichier à jeton de contexte.
///
/// ```
/// use std::collections::BTreeSet;
/// use ygo_app::galerie::{fichier_pour, Reprise};
/// let reprise = Reprise {
///     rowid: 976,
///     numero: "RA04-EN108".into(),
///     abbr: "QCSCR".into(),
///     suffixe: "AA2".into(),
///     cles: ["blueeyeswhitedragon".to_owned()].into(),
///     segments: BTreeSet::new(),
///     edition: None,
///     avant: None,
/// };
/// let scans = [
///     ("BlueEyesWhiteDragon-RA04-EN-QCScR-1E-AA.png".to_owned(), "u1".to_owned()),
///     ("BlueEyesWhiteDragon-RA04-EN-QCScR-1E-AA2.png".to_owned(), "u2".to_owned()),
///     ("BlueEyesWhiteDragon-RA04-EN-QCScR-1E-AA2-OP.png".to_owned(), "u3".to_owned()),
/// ];
/// assert_eq!(fichier_pour(&reprise, &scans, "RA04", "EN").map(|s| s.1.as_str()), Some("u2"));
/// ```
#[must_use]
pub fn fichier_pour<'a>(
    reprise: &Reprise,
    scans: &'a [(String, String)],
    prefixe: &str,
    langue: &str,
) -> Option<&'a (String, String)> {
    let voulue = edition_fichier(reprise.edition.as_deref());
    let rang = |edition: &str| {
        if voulue.is_some_and(|v| v.eq_ignore_ascii_case(edition)) {
            return 0;
        }
        ["1E", "", "UE", "LE"]
            .iter()
            .position(|e| e.eq_ignore_ascii_case(edition))
            .map_or(9, |p| p + 1)
    };
    scans
        .iter()
        .filter_map(|scan| {
            let (segment, infos) = artwork::decouper_fichier(&scan.0, prefixe)?;
            let convient = reprise.cles.contains(&artwork::cle_comparaison(&segment))
                && infos.rarete_abbr.eq_ignore_ascii_case(&reprise.abbr)
                && artwork::suffixe(&scan.0, prefixe).eq_ignore_ascii_case(&reprise.suffixe)
                && !artwork::porte_un_contexte(&scan.0)
                && (infos.langue.is_empty() || infos.langue.eq_ignore_ascii_case(langue))
                && !scan.1.is_empty();
            convient.then(|| (rang(&infos.edition), scan))
        })
        .min_by(|(ra, a), (rb, b)| (ra, &a.0).cmp(&(rb, &b.0)))
        .map(|(_, scan)| scan)
}

/// Lit les lignes d'un classeur, son nom de set et les choix de
/// l'utilisateur.
///
/// # Errors
///
/// Rend une erreur si le classeur est illisible.
pub fn lire(conn: &Connection) -> Result<(Vec<Ligne>, String, HashSet<i64>)> {
    let ext = if colonne_existe(conn, "extended_art")? {
        "COALESCE(extended_art,0)"
    } else {
        "0"
    };
    let edition = if colonne_existe(conn, "edition")? {
        "edition"
    } else {
        "NULL"
    };
    let sql = format!(
        "SELECT rowid, COALESCE(name,''), UPPER(COALESCE(set_code,'')), COALESCE(rarity,''), \
                {ext}, {edition}, COALESCE(quantite,0), COALESCE(card_image_url,''), \
                COALESCE(card_image_uuid,''), COALESCE(set_name,'') \
           FROM cards WHERE COALESCE(name,'') <> '' ORDER BY rowid"
    );
    let mut requete = conn.prepare(&sql)?;
    let mut set_name = String::new();
    let lignes = requete
        .query_map([], |l| {
            Ok((
                Ligne {
                    rowid: l.get(0)?,
                    name: l.get(1)?,
                    set_code: l.get(2)?,
                    rarity: l.get(3)?,
                    overframe: l.get::<_, i64>(4)? == 1,
                    edition: l
                        .get::<_, Option<String>>(5)?
                        .filter(|e| !e.trim().is_empty()),
                    quantite: l.get(6)?,
                    url: l.get(7)?,
                    uuid: l.get(8)?,
                },
                l.get::<_, String>(9)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .map(|(ligne, nom)| {
            if set_name.is_empty() && !nom.trim().is_empty() {
                nom.trim().clone_into(&mut set_name);
            }
            ligne
        })
        .collect();
    Ok((lignes, set_name, choix(conn)?))
}

/// La table `meta` existe-t-elle ? Un classeur très ancien peut ne pas
/// l'avoir.
fn a_meta(conn: &Connection) -> Result<bool> {
    Ok(conn
        .prepare("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'meta'")?
        .exists([])?)
}

/// Les lignes dont l'utilisateur a choisi l'image.
///
/// # Errors
///
/// Rend une erreur si la table `meta` est illisible.
pub fn choix(conn: &Connection) -> Result<HashSet<i64>> {
    if !a_meta(conn)? {
        return Ok(HashSet::new());
    }
    let mut requete = conn.prepare("SELECT key FROM meta WHERE key LIKE ?1")?;
    let cles = requete
        .query_map([format!("{CLE_CHOIX}%")], |l| l.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(cles
        .iter()
        .filter_map(|c| c.strip_prefix(CLE_CHOIX)?.parse().ok())
        .collect())
}

/// Note que l'utilisateur a choisi l'image de cette ligne — `fichier`, ou
/// `None` pour oublier le choix (« Annuler »).
///
/// # Errors
///
/// Rend une erreur si la table `meta` ne peut pas être écrite.
pub fn noter_choix(conn: &Connection, rowid: i64, fichier: Option<&str>) -> Result<()> {
    if !a_meta(conn)? {
        if fichier.is_none() {
            return Ok(());
        }
        conn.execute_batch("CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT)")?;
    }
    let cle = format!("{CLE_CHOIX}{rowid}");
    match fichier {
        Some(f) => conn.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
            (&cle, f),
        )?,
        None => conn.execute("DELETE FROM meta WHERE key = ?1", [&cle])?,
    };
    Ok(())
}

/// Les scans du set que l'installation connaît sans réseau : ceux des lignes
/// du classeur, et les fichiers de `img/small`.
#[must_use]
pub fn scans_locaux(paths: &Paths, lignes: &[Ligne], prefixe: &str) -> Vec<(String, String)> {
    let mut scans: BTreeMap<String, String> = BTreeMap::new();
    for l in lignes {
        if let Some(fichier) = scan_de(&l.url) {
            let url =
                if ygo_images::plan::nom_de_fichier(&l.url).as_deref() == Some(fichier.as_str()) {
                    l.url.clone()
                } else {
                    artwork::url_fichier(&fichier)
                };
            scans.entry(fichier).or_insert(url);
        }
    }
    let marqueur = format!("-{prefixe}-").to_ascii_uppercase();
    if let Ok(entrees) = std::fs::read_dir(paths.img_small()) {
        for nom in entrees
            .filter_map(std::result::Result::ok)
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| n.to_ascii_uppercase().contains(&marqueur))
        {
            let url = artwork::url_fichier(&nom);
            scans.entry(nom).or_insert(url);
        }
    }
    scans.into_iter().collect()
}

/// Pose les scans retenus — même écriture que la passe artworks : l'uuid
/// `yugipedia:` protège la ligne des passes suivantes et d'`images_tirage`.
/// Possession, quantité, état et édition ne bougent pas.
///
/// # Errors
///
/// Rend une erreur si l'écriture échoue.
pub fn ecrire(conn: &mut Connection, poses: &[(i64, String, String)]) -> Result<usize> {
    let transaction = conn.transaction()?;
    let mut n = 0;
    for (rowid, fichier, url) in poses {
        n += transaction.execute(
            "UPDATE cards SET card_image_uuid = ?1, card_image_id = ?2, \
                    card_image_url = ?3, card_image_small = ?3 \
              WHERE rowid = ?4",
            rusqlite::params![
                format!("{PREFIXE_SCAN}{fichier}"),
                artwork::id_synthetique(fichier),
                url,
                rowid
            ],
        )?;
    }
    transaction.commit()?;
    Ok(n)
}

/// La langue de la plupart des numéros du classeur — `EN` à défaut : un
/// tirage anglais rangé dans un classeur japonais (`CORI-ENS10`) ne doit pas
/// faire lire la galerie anglaise.
fn langue_majoritaire(lignes: &[Ligne]) -> String {
    let mut comptes: BTreeMap<String, usize> = BTreeMap::new();
    for l in lignes {
        let langue = artwork::langue_set_code(&l.set_code);
        if !langue.is_empty() {
            *comptes.entry(langue).or_default() += 1;
        }
    }
    comptes
        .into_iter()
        .max_by_key(|(langue, n)| (*n, Reverse(langue.clone())))
        .map_or_else(|| "EN".to_owned(), |(langue, _)| langue)
}

/// Ce qu'un réalignement a fait.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bilan {
    /// Les pages de galerie lues — vide : Yugipedia n'en a pas pour ce set.
    pub titres: Vec<String>,
    /// Lignes que la galerie contredit.
    pub reprises: usize,
    /// Lignes qui ont reçu leur scan.
    pub posees: usize,
    /// Lignes dont le scan attendu n'a pas été trouvé — elles restent comme
    /// elles étaient.
    pub sans_fichier: usize,
}

/// Les scans à poser : ceux des reprises qu'on sait illustrer.
fn a_poser(
    reprises: &[Reprise],
    scans: &[(String, String)],
    prefixe: &str,
    langue: &str,
) -> (Vec<(i64, String, String)>, usize) {
    let mut poses = Vec::new();
    let mut sans = 0;
    for r in reprises {
        match fichier_pour(r, scans, prefixe, langue) {
            Some((fichier, url)) => poses.push((r.rowid, fichier.clone(), url.clone())),
            None => sans += 1,
        }
    }
    (poses, sans)
}

/// Réaligne un classeur sur la galerie de son set.
///
/// Deux requêtes pour la galerie la première fois, gardées 30 jours ; une
/// requête `allimages` par carte dont le scan attendu n'est connu ni des
/// lignes ni de `img/small` — aucune quand rien n'est à reprendre.
///
/// # Errors
///
/// Rend l'erreur du réseau ou de SQLite ; un classeur absent ou vide n'en est
/// pas une.
pub async fn realigner(
    paths: &Paths,
    client: &ygo_sources::ClientHttp,
    classeur: &str,
) -> Result<Bilan> {
    let chemin = paths.classeur_db(classeur);
    if !chemin.is_file() {
        return Ok(Bilan::default());
    }
    let (lignes, set_name, choix) = {
        let conn = ygo_db::connexion::ouvrir_lecture_seule(&chemin)?;
        lire(&conn)?
    };
    let Some(premiere) = lignes.first() else {
        return Ok(Bilan::default());
    };
    if set_name.is_empty() {
        return Ok(Bilan::default());
    }
    let prefixe = artwork::prefixe_set(&premiere.set_code);
    let langue = langue_majoritaire(&lignes);
    let galerie = ygo_sources::yugipedia::galerie::lire(client, &set_name, &langue).await?;
    let mut bilan = Bilan {
        titres: galerie.titres.clone(),
        ..Bilan::default()
    };
    if galerie.est_vide() {
        return Ok(bilan);
    }
    let reprises = planifier(&lignes, &galerie, &prefixe, &choix);
    bilan.reprises = reprises.len();
    if reprises.is_empty() {
        return Ok(bilan);
    }

    let mut scans = scans_locaux(paths, &lignes, &prefixe);
    let manquants: BTreeSet<String> = reprises
        .iter()
        .filter(|r| fichier_pour(r, &scans, &prefixe, &langue).is_none())
        .flat_map(|r| r.segments.iter().cloned())
        .collect();
    let mut connus: HashSet<String> = scans.iter().map(|(f, _)| f.clone()).collect();
    for segment in manquants {
        for f in artwork::lister_images(client, &format!("{segment}-{prefixe}-")).await? {
            let fichier = ygo_images::plan::nom_de_fichier(&f.url).unwrap_or(f.nom);
            if connus.insert(fichier.clone()) {
                scans.push((fichier, f.url));
            }
        }
    }

    let (poses, sans_fichier) = a_poser(&reprises, &scans, &prefixe, &langue);
    bilan.sans_fichier = sans_fichier;
    if !poses.is_empty() {
        let mut conn = ygo_db::connexion::ouvrir(&chemin)?;
        bilan.posees = ecrire(&mut conn, &poses)?;
    }
    for r in &reprises {
        tracing::info!(
            classeur,
            numero = %r.numero,
            rarete = %r.abbr,
            rowid = r.rowid,
            avant = r.avant.as_deref().unwrap_or("—"),
            attendu = if r.suffixe.is_empty() { "nu" } else { r.suffixe.as_str() },
            "scan rendu à son numéro d'après la galerie"
        );
    }
    tracing::info!(
        classeur,
        galerie = %bilan.titres.join(" + "),
        reprises = bilan.reprises,
        posees = bilan.posees,
        sans_fichier = bilan.sans_fichier,
        "classeur réaligné sur la galerie Yugipedia"
    );
    Ok(bilan)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;
    use ygo_sources::yugipedia::galerie::parse_galerie;

    const RA04: &str =
        include_str!("../../../tests/fixtures/net/yugipedia/galerie_RA04-EN-1E_extrait.txt");

    fn galerie() -> Galerie {
        Galerie {
            titres: vec!["Set Card Galleries:Quarter Century Stampede (TCG-EN-1E)".into()],
            entrees: parse_galerie(RA04),
        }
    }

    fn url(fichier: &str) -> String {
        format!("https://ms.yugipedia.com//a/ab/{fichier}")
    }

    /// Une ligne de `RA04`, servie par une passe (`yugipedia:`) ou posée par
    /// `images_tirage` (uuid YGOJSON), possédée ou non.
    fn ligne(
        rowid: i64,
        numero: &str,
        rarete: &str,
        fichier: &str,
        servie: bool,
        quantite: i64,
    ) -> Ligne {
        let nom = if numero == "RA04-EN002" || numero == "RA04-EN108" {
            "Blue-Eyes White Dragon"
        } else {
            "Dark Magician"
        };
        Ligne {
            rowid,
            name: nom.into(),
            set_code: numero.into(),
            rarity: rarete.into(),
            overframe: false,
            edition: Some("1st".into()),
            quantite,
            url: if fichier.is_empty() {
                String::new()
            } else {
                url(fichier)
            },
            uuid: if servie {
                format!("yugipedia:{fichier}")
            } else {
                "86c4f121-7be3-40ed-972e-7592310782a7".into()
            },
        }
    }

    const QC: &str = "Quarter Century Secret Rare";
    const PL: &str = "Platinum Secret Rare";

    /// L'état réel de « Projet Clean » (2026-10-18) : chaque contradiction
    /// relevée trouve sa reprise, et rien d'autre ne bouge.
    fn projet_clean() -> Vec<Ligne> {
        vec![
            ligne(
                983,
                "RA04-EN001",
                QC,
                "DarkMagician-RA04-EN-QCScR-1E-AA.png",
                true,
                0,
            ),
            ligne(
                984,
                "RA04-EN002",
                QC,
                "BlueEyesWhiteDragon-RA04-EN-QCScR-1E-AA.png",
                true,
                1,
            ),
            ligne(
                965,
                "RA04-EN106",
                PL,
                "DarkMagician-RA04-EN-PlScR-1E-AA.png",
                true,
                0,
            ),
            ligne(
                966,
                "RA04-EN106",
                PL,
                "DarkMagician-RA04-EN-PlScR-1E-AA2.png",
                true,
                4,
            ),
            ligne(
                967,
                "RA04-EN106",
                PL,
                "DarkMagician-RA04-EN-PlScR-1E-AA3.png",
                true,
                0,
            ),
            ligne(
                968,
                "RA04-EN106",
                PL,
                "DarkMagician-RA04-EN-PlScR-1E-AA4.png",
                true,
                0,
            ),
            ligne(
                582,
                "RA04-EN106",
                PL,
                "DarkMagician-RA04-EN-PlScR-1E-AA2.png",
                false,
                0,
            ),
            ligne(
                969,
                "RA04-EN106",
                QC,
                "DarkMagician-RA04-EN-QCScR-1E-AA.png",
                true,
                0,
            ),
            ligne(
                970,
                "RA04-EN106",
                QC,
                "DarkMagician-RA04-EN-QCScR-1E-AA2.png",
                true,
                0,
            ),
            ligne(
                971,
                "RA04-EN106",
                QC,
                "DarkMagician-RA04-EN-QCScR-1E-AA3.png",
                true,
                0,
            ),
            ligne(
                972,
                "RA04-EN106",
                QC,
                "DarkMagician-RA04-EN-QCScR-1E-AA4.png",
                true,
                0,
            ),
            ligne(
                583,
                "RA04-EN106",
                QC,
                "DarkMagician-RA04-EN-QCScR-1E.png",
                false,
                0,
            ),
            ligne(
                975,
                "RA04-EN108",
                PL,
                "BlueEyesWhiteDragon-RA04-EN-PlScR-1E.png",
                true,
                0,
            ),
            ligne(
                586,
                "RA04-EN108",
                PL,
                "BlueEyesWhiteDragon-RA04-EN-PlScR-1E-AA.png",
                true,
                0,
            ),
            ligne(
                976,
                "RA04-EN108",
                QC,
                "BlueEyesWhiteDragon-RA04-EN-QCScR-1E.png",
                true,
                0,
            ),
            ligne(
                587,
                "RA04-EN108",
                QC,
                "BlueEyesWhiteDragon-RA04-EN-QCScR-1E-AA.png",
                true,
                0,
            ),
            // Une carte que la galerie décrit sans désaccord : son image
            // YGOPRODeck face au scan nu.
            Ligne {
                url: "https://images.ygoprodeck.com/images/cards/123.jpg".into(),
                ..ligne(5, "RA04-EN005", "Super Rare", "", false, 1)
            },
        ]
    }

    fn resume(reprises: &[Reprise]) -> Vec<(i64, &str)> {
        let mut v: Vec<(i64, &str)> = reprises
            .iter()
            .map(|r| (r.rowid, r.suffixe.as_str()))
            .collect();
        v.sort_unstable();
        v
    }

    #[test]
    fn le_cas_ra04_de_projet_clean() {
        let mut lignes = projet_clean();
        let reprises = planifier(&lignes, &galerie(), "RA04", &HashSet::new());
        assert_eq!(
            resume(&reprises),
            vec![
                (582, ""),    // la Platinum d'EN106 qui doublait l'AA2 : le scan nu
                (583, "AA5"), // la QCScR d'EN106 qui montrait le nu d'EN001
                (976, "AA2"), // la QCScR d'EN108 qui montrait le nu d'EN002
                (983, ""),    // EN001 : son scan nu
                (984, ""),    // EN002 : son scan nu — la carte en main
            ]
        );
        let r = reprises.iter().find(|r| r.rowid == 984).unwrap();
        assert_eq!(
            r.avant.as_deref(),
            Some("BlueEyesWhiteDragon-RA04-EN-QCScR-1E-AA.png")
        );
        assert!(r.segments.contains("BlueEyesWhiteDragon"));
        let arkana = reprises.iter().find(|r| r.rowid == 582).unwrap();
        assert!(arkana.cles.contains("darkmagician"));
        assert!(
            arkana.segments.contains("DarkMagician"),
            "{:?}",
            arkana.segments
        );

        // Une fois les scans posés, plus rien à reprendre : c'est un point
        // fixe.
        for r in &reprises {
            let l = lignes.iter_mut().find(|l| l.rowid == r.rowid).unwrap();
            let carte = if l.name.starts_with("Blue") {
                "BlueEyesWhiteDragon"
            } else {
                "DarkMagician"
            };
            let abbr = if l.rarity == QC { "QCScR" } else { "PlScR" };
            let fin = if r.suffixe.is_empty() {
                String::new()
            } else {
                format!("-{}", r.suffixe)
            };
            l.url = url(&format!("{carte}-RA04-EN-{abbr}-1E{fin}.png"));
            l.uuid = "yugipedia:x".into();
        }
        assert!(planifier(&lignes, &galerie(), "RA04", &HashSet::new()).is_empty());
    }

    /// La ligne possédée garde son image quand deux lignes montrent la même
    /// illustration — même venue après l'autre ; à possession égale, celle
    /// qu'une passe a servie.
    #[test]
    fn de_deux_lignes_jumelles_c_est_l_autre_qui_change() {
        let lignes = vec![
            ligne(
                582,
                "RA04-EN106",
                PL,
                "DarkMagician-RA04-EN-PlScR-1E-AA2.png",
                true,
                0,
            ),
            ligne(
                966,
                "RA04-EN106",
                PL,
                "DarkMagician-RA04-EN-PlScR-1E-AA2.png",
                true,
                4,
            ),
        ];
        let r = planifier(&lignes, &galerie(), "RA04", &HashSet::new());
        assert_eq!(
            resume(&r),
            vec![(582, "")],
            "la possédée (966) garde son AA2"
        );

        let lignes = vec![
            ligne(
                582,
                "RA04-EN106",
                PL,
                "DarkMagician-RA04-EN-PlScR-1E-AA2.png",
                false,
                0,
            ),
            ligne(
                966,
                "RA04-EN106",
                PL,
                "DarkMagician-RA04-EN-PlScR-1E-AA2.png",
                true,
                0,
            ),
        ];
        let r = planifier(&lignes, &galerie(), "RA04", &HashSet::new());
        assert_eq!(resume(&r), vec![(582, "")], "la servie (966) garde le sien");
    }

    /// Le choix de l'utilisateur l'emporte sur la galerie — et passe avant
    /// une ligne possédée qui montre la même illustration : c'est elle qui
    /// prend celle qui reste.
    #[test]
    fn un_choix_de_l_utilisateur_n_est_jamais_realigne() {
        let lignes = vec![ligne(
            984,
            "RA04-EN002",
            QC,
            "BlueEyesWhiteDragon-RA04-EN-QCScR-1E-AA.png",
            true,
            1,
        )];
        let choix: HashSet<i64> = [984].into();
        assert!(planifier(&lignes, &galerie(), "RA04", &choix).is_empty());

        let aa = "BlueEyesWhiteDragon-RA04-EN-QCScR-1E-AA.png";
        let lignes = vec![
            ligne(976, "RA04-EN108", QC, aa, true, 0),
            ligne(587, "RA04-EN108", QC, aa, true, 1),
        ];
        let choix: HashSet<i64> = [976].into();
        let r = planifier(&lignes, &galerie(), "RA04", &choix);
        assert_eq!(resume(&r), vec![(587, "AA2")], "976, choisie, garde son AA");
    }

    /// La langue du classeur est celle de la plupart de ses numéros.
    #[test]
    fn la_langue_du_classeur_est_celle_de_ses_numeros() {
        let numero = |rowid: i64, code: &str| Ligne {
            rowid,
            set_code: code.into(),
            ..Ligne::default()
        };
        let cori = [
            numero(1, "CORI-ENS10"),
            numero(2, "CORI-JP001"),
            numero(3, "CORI-JPS10"),
        ];
        assert_eq!(langue_majoritaire(&cori), "JP");
        assert_eq!(langue_majoritaire(&[numero(1, "RA04-EN001")]), "EN");
        let egalite = [numero(1, "CORI-JP001"), numero(2, "CORI-EN001")];
        assert_eq!(
            langue_majoritaire(&egalite),
            "EN",
            "égalité : l'ordre alphabétique"
        );
        assert_eq!(langue_majoritaire(&[numero(1, "PROMO")]), "EN", "à défaut");
        assert_eq!(langue_majoritaire(&[]), "EN");
    }

    /// Une image YGOPRODeck face au scan nu reste ; face à une illustration
    /// alternative, elle reçoit le scan — YGOPRODeck ne peut pas la montrer.
    /// Une ligne sans image reçoit le sien.
    #[test]
    fn une_image_ygoprodeck_ne_cede_qu_a_une_illustration_alternative() {
        let ygo = "https://images.ygoprodeck.com/images/cards/46986414.jpg";
        let mut nu = ligne(983, "RA04-EN001", QC, "", false, 0);
        nu.url = ygo.into();
        assert!(planifier(&[nu.clone()], &galerie(), "RA04", &HashSet::new()).is_empty());

        let mut alternative = ligne(976, "RA04-EN108", QC, "", false, 0);
        alternative.url = ygo.into();
        let r = planifier(&[alternative], &galerie(), "RA04", &HashSet::new());
        assert_eq!(resume(&r), vec![(976, "AA")]);
        assert_eq!(r[0].avant, None);

        let sans = ligne(983, "RA04-EN001", QC, "", false, 0);
        assert_eq!(
            resume(&planifier(&[sans], &galerie(), "RA04", &HashSet::new())),
            vec![(983, "")]
        );
    }

    /// Une ligne Overframe ne reçoit qu'un `EA`, une normale jamais ; une
    /// ligne en trop reste comme elle est ; une rareté que la galerie ne
    /// montre pas n'est pas touchée.
    #[test]
    fn le_cadre_et_les_lignes_en_trop_sont_respectes() {
        let mut overframe = ligne(
            976,
            "RA04-EN108",
            QC,
            "BlueEyesWhiteDragon-RA04-EN-QCScR-1E.png",
            true,
            0,
        );
        overframe.overframe = true;
        assert!(
            planifier(&[overframe], &galerie(), "RA04", &HashSet::new()).is_empty(),
            "aucun EA attendu : l'Overframe garde son image"
        );
        let trois = vec![
            ligne(
                1,
                "RA04-EN002",
                QC,
                "BlueEyesWhiteDragon-RA04-EN-QCScR-1E.png",
                true,
                0,
            ),
            ligne(
                2,
                "RA04-EN002",
                QC,
                "BlueEyesWhiteDragon-RA04-EN-QCScR-1E-AA.png",
                true,
                0,
            ),
        ];
        assert!(
            planifier(&trois, &galerie(), "RA04", &HashSet::new()).is_empty(),
            "une illustration, deux lignes : la seconde reste"
        );
        let ur = ligne(
            1,
            "RA04-EN002",
            "Ultra Rare",
            "BlueEyesWhiteDragon-RA04-EN-UR-1E-AA.png",
            true,
            0,
        );
        assert!(planifier(&[ur], &galerie(), "RA04", &HashSet::new()).is_empty());
        assert!(planifier(
            &projet_clean(),
            &Galerie::default(),
            "RA04",
            &HashSet::new()
        )
        .is_empty());
    }

    /// Le scan d'une autre carte sur la ligne est une contradiction, comme
    /// celui d'une autre illustration.
    #[test]
    fn le_scan_d_une_autre_carte_est_repris() {
        let l = ligne(
            984,
            "RA04-EN002",
            QC,
            "DarkMagician-RA04-EN-QCScR-1E.png",
            true,
            0,
        );
        let r = planifier(&[l], &galerie(), "RA04", &HashSet::new());
        assert_eq!(resume(&r), vec![(984, "")]);
        assert_eq!(
            r[0].avant.as_deref(),
            Some("DarkMagician-RA04-EN-QCScR-1E.png")
        );
    }

    /// Le fichier : l'édition de la ligne d'abord, le bon suffixe seulement.
    #[test]
    fn le_fichier_suit_l_edition_de_la_ligne() {
        let mut reprise = planifier(
            &[ligne(
                984,
                "RA04-EN002",
                QC,
                "BlueEyesWhiteDragon-RA04-EN-QCScR-1E-AA.png",
                true,
                0,
            )],
            &galerie(),
            "RA04",
            &HashSet::new(),
        )
        .remove(0);
        let scans: Vec<(String, String)> = [
            "BlueEyesWhiteDragon-RA04-EN-QCScR-1E-AA.png",
            "BlueEyesWhiteDragon-RA04-EN-QCScR-UE.png",
            "BlueEyesWhiteDragon-RA04-EN-QCScR-1E.png",
            "BlueEyesWhiteDragon-RA04-FR-QCScR-1E.png",
        ]
        .iter()
        .map(|f| ((*f).to_owned(), url(f)))
        .collect();
        let choisi = |r: &Reprise| fichier_pour(r, &scans, "RA04", "EN").map(|s| s.0.clone());
        assert_eq!(
            choisi(&reprise).as_deref(),
            Some("BlueEyesWhiteDragon-RA04-EN-QCScR-1E.png")
        );
        reprise.edition = Some("unlimited".into());
        assert_eq!(
            choisi(&reprise).as_deref(),
            Some("BlueEyesWhiteDragon-RA04-EN-QCScR-UE.png")
        );
        reprise.suffixe = "AA9".into();
        assert_eq!(choisi(&reprise), None);
    }

    /// Une installation jetable : le classeur `RA04` de « Projet Clean ».
    fn installation() -> (tempfile::TempDir, Paths) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(tmp.path());
        std::fs::create_dir_all(paths.dossier_classeur("RA04")).unwrap();
        let conn = rusqlite::Connection::open(paths.classeur_db("RA04")).unwrap();
        conn.execute_batch(ygo_db::schema::DDL_CLASSEUR).unwrap();
        for l in projet_clean() {
            conn.execute(
                "INSERT INTO cards (rowid, name, set_code, rarity, set_name, card_image_url, \
                                    card_image_uuid, possessed, quantite, edition, extended_art) \
                 VALUES (?1, ?2, ?3, ?4, 'Quarter Century Stampede', ?5, ?6, ?7, ?8, '1st', 0)",
                rusqlite::params![
                    l.rowid,
                    l.name,
                    l.set_code,
                    l.rarity,
                    l.url,
                    l.uuid,
                    i64::from(l.quantite > 0),
                    l.quantite
                ],
            )
            .unwrap();
        }
        (tmp, paths)
    }

    /// De la base à la base : lire, planifier, trouver les fichiers, écrire —
    /// et la quantité de la carte en main ne bouge pas.
    #[test]
    fn le_realignement_ecrit_les_scans_et_rien_d_autre() {
        let (_tmp, paths) = installation();
        let mut conn = rusqlite::Connection::open(paths.classeur_db("RA04")).unwrap();
        let (lignes, set_name, choix) = lire(&conn).unwrap();
        assert_eq!(set_name, "Quarter Century Stampede");
        assert!(choix.is_empty());
        let reprises = planifier(&lignes, &galerie(), "RA04", &choix);
        assert_eq!(reprises.len(), 5);
        // Les fichiers : ceux des lignes, plus ce qu'`allimages` aurait rendu.
        let mut scans = scans_locaux(&paths, &lignes, "RA04");
        for f in [
            "DarkMagician-RA04-EN-PlScR-1E.png",
            "DarkMagician-RA04-EN-QCScR-1E-AA5.png",
            "BlueEyesWhiteDragon-RA04-EN-QCScR-1E-AA2.png",
        ] {
            scans.push((f.to_owned(), url(f)));
        }
        let (poses, sans) = a_poser(&reprises, &scans, "RA04", "EN");
        assert_eq!(sans, 0);
        assert_eq!(ecrire(&mut conn, &poses).unwrap(), 5);

        let lu = |rowid: i64| -> (String, Option<i64>, String, i64) {
            conn.query_row(
                "SELECT card_image_uuid, card_image_id, card_image_url, quantite FROM cards WHERE rowid = ?1",
                [rowid],
                |l| Ok((l.get(0)?, l.get(1)?, l.get(2)?, l.get(3)?)),
            )
            .unwrap()
        };
        let (uuid, id, adresse, quantite) = lu(984);
        assert_eq!(uuid, "yugipedia:BlueEyesWhiteDragon-RA04-EN-QCScR-1E.png");
        assert_eq!(
            id,
            Some(artwork::id_synthetique(
                "BlueEyesWhiteDragon-RA04-EN-QCScR-1E.png"
            ))
        );
        assert!(adresse.ends_with("/BlueEyesWhiteDragon-RA04-EN-QCScR-1E.png"));
        assert_eq!(quantite, 1, "la carte en main garde son exemplaire");
        assert_eq!(
            lu(976).0,
            "yugipedia:BlueEyesWhiteDragon-RA04-EN-QCScR-1E-AA2.png"
        );
        assert_eq!(lu(583).0, "yugipedia:DarkMagician-RA04-EN-QCScR-1E-AA5.png");
        assert_eq!(
            lu(966).0,
            "yugipedia:DarkMagician-RA04-EN-PlScR-1E-AA2.png",
            "intacte"
        );

        // Relu, le classeur ne contredit plus la galerie.
        let (lignes, _, choix) = lire(&conn).unwrap();
        assert!(planifier(&lignes, &galerie(), "RA04", &choix).is_empty());
    }

    /// Un choix noté se relit, et s'oublie.
    #[test]
    fn un_choix_se_note_et_s_oublie() {
        let (_tmp, paths) = installation();
        let conn = rusqlite::Connection::open(paths.classeur_db("RA04")).unwrap();
        noter_choix(
            &conn,
            984,
            Some("BlueEyesWhiteDragon-RA04-EN-QCScR-1E-AA.png"),
        )
        .unwrap();
        noter_choix(&conn, 976, Some("x.png")).unwrap();
        assert_eq!(choix(&conn).unwrap(), [984, 976].into());
        noter_choix(&conn, 976, None).unwrap();
        assert_eq!(choix(&conn).unwrap(), [984].into());
        let (lignes, _, choisies) = lire(&conn).unwrap();
        let rowids: Vec<i64> = planifier(&lignes, &galerie(), "RA04", &choisies)
            .iter()
            .map(|r| r.rowid)
            .collect();
        assert!(!rowids.contains(&984), "{rowids:?}");
        assert!(rowids.contains(&976));

        // Sans table `meta`, rien n'est choisi, et oublier ne la crée pas.
        let nue = rusqlite::Connection::open_in_memory().unwrap();
        assert!(choix(&nue).unwrap().is_empty());
        noter_choix(&nue, 1, None).unwrap();
        assert!(!a_meta(&nue).unwrap());
        noter_choix(&nue, 1, Some("f.png")).unwrap();
        assert_eq!(choix(&nue).unwrap(), [1].into());
    }
}
