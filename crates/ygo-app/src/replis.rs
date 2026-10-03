// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Les images servies par la source de **repli**, et leur reprise à chaque
//! mise à jour de la base.
//!
//! # Le constat — 2026-10-01
//!
//! Sur `LOCR-JP`, 27 images de tirage (9 cartes × UR/ScR/PScR) ont reçu un 404
//! de Yugipedia : YGOJSON annonce des noms de fichiers que le wiki n'a pas. Le
//! téléchargeur s'est replié sur l'image YGOPRODeck de la carte — c'est son
//! rôle — et l'a écrite **sous le nom du fichier Yugipedia**. Le fichier étant
//! présent, aucune ouverture ne le retentait jamais, même si le wiki ou
//! YGOJSON corrigeaient un jour le nom.
//!
//! # Le déclencheur — demande de l'utilisateur du 2026-10-02
//!
//! « Une tentative à chaque mise à jour, puisque l'application est mise à jour
//! par les mises à jour de l'API. » C'est le bon moment : une nouvelle base
//! peut porter une nouvelle adresse d'image, et c'est le seul événement qui
//! le peut. La reprise ([`reprendre`]) fait deux choses, dans cet ordre :
//!
//! 1. **reposer les images de tirage** de chaque classeur depuis la nouvelle
//!    base ([`crate::images_tirage::poser`]) — si le nom a été corrigé, la
//!    ligne reçoit la nouvelle adresse ;
//! 2. **vérifier, puis rouvrir** : demander à Yugipedia, en **une** requête
//!    pour 50 fichiers (`Yugipedia:API`, *bundled together … with a pipe*),
//!    lesquels il a désormais ; n'effacer l'image de repli et n'oublier le 404
//!    que de ceux-là. La vraie image arrive à la prochaine ouverture du
//!    classeur, une fois — ensuite ce n'est plus un repli. Les absents gardent
//!    leur image de repli et restent au registre (amélioration du 2026-10-02 :
//!    la première version retéléchargeait les 27 à chaque mise à jour, pour 27
//!    404 probables).
//!
//! # Deux façons de connaître un repli
//!
//! - le **registre** `img/replis.json`, tenu depuis le 2026-10-02 par
//!   [`consigner`] après chaque passe d'images ;
//! - la **détection** ([`detecter`]) pour ceux d'avant : un fichier nommé
//!   `….png` d'après une adresse Yugipedia, mais dont le contenu est un JPEG —
//!   le format des images YGOPRODeck. C'est la signature exacte des 27.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use ygo_core::paths::Paths;
use ygo_core::rarity::Priorites;

use crate::error::Result;

/// Une image servie par la source de repli.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Repli {
    /// Le fichier, relatif au dossier `img/` quand il s'y trouve.
    pub destination: String,
    /// L'adresse de la source primaire, qui a échoué.
    pub url: String,
}

/// Le registre des replis.
fn registre(paths: &Paths) -> PathBuf {
    paths.img().join("replis.json")
}

/// Le chemin tel qu'on le garde : relatif à `img/`, en `/`.
fn relatif(paths: &Paths, chemin: &Path) -> String {
    chemin
        .strip_prefix(paths.img())
        .unwrap_or(chemin)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Le chemin réel d'une destination gardée.
fn absolu(paths: &Paths, destination: &str) -> PathBuf {
    let chemin = Path::new(destination);
    if chemin.is_absolute() {
        chemin.to_path_buf()
    } else {
        paths.img().join(chemin)
    }
}

/// Le registre, vide s'il n'existe pas ou ne se lit pas.
#[must_use]
pub fn lire(paths: &Paths) -> Vec<Repli> {
    std::fs::read(registre(paths))
        .ok()
        .and_then(|o| serde_json::from_slice(&o).ok())
        .unwrap_or_default()
}

fn ecrire(paths: &Paths, replis: &[Repli]) -> Result<()> {
    let chemin = registre(paths);
    if replis.is_empty() {
        if chemin.is_file() {
            std::fs::remove_file(&chemin)?;
        }
        return Ok(());
    }
    if let Some(parent) = chemin.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let provisoire = chemin.with_extension("tmp");
    std::fs::write(&provisoire, serde_json::to_vec_pretty(replis)?)?;
    std::fs::rename(&provisoire, &chemin)?;
    Ok(())
}

/// Ajoute au registre les replis d'une passe : `(destination, URL primaire)`.
///
/// Une même destination n'y figure qu'une fois — la dernière adresse gagne.
/// Ne lève jamais : un registre non écrit coûte une reprise, pas une image.
pub fn consigner(paths: &Paths, nouveaux: &[(PathBuf, String)]) {
    if nouveaux.is_empty() {
        return;
    }
    let mut tous: BTreeMap<String, String> = lire(paths)
        .into_iter()
        .map(|r| (r.destination, r.url))
        .collect();
    for (destination, url) in nouveaux {
        tous.insert(relatif(paths, destination), url.clone());
    }
    let replis: Vec<Repli> = tous
        .into_iter()
        .map(|(destination, url)| Repli { destination, url })
        .collect();
    match ecrire(paths, &replis) {
        Ok(()) => tracing::info!(
            nouveaux = nouveaux.len(),
            total = replis.len(),
            "images servies par la source de repli consignées"
        ),
        Err(e) => tracing::warn!(erreur = %e, "registre des replis non écrit"),
    }
}

/// Le contenu est-il un JPEG ?
fn est_jpeg(chemin: &Path) -> bool {
    use std::io::Read;
    let mut tete = [0_u8; 3];
    std::fs::File::open(chemin)
        .and_then(|mut f| f.read_exact(&mut tete))
        .is_ok()
        && tete == [0xFF, 0xD8, 0xFF]
}

/// Les replis d'avant le registre : une image nommée `….png` d'après une
/// adresse Yugipedia, dont le contenu est un JPEG.
///
/// La destination suit la règle du planificateur
/// ([`ygo_images::planifier`]) : `img/small` quand la ligne a un
/// identifiant d'image, le dossier du classeur sinon.
///
/// # Errors
///
/// Rend une erreur si un classeur est illisible.
pub fn detecter(paths: &Paths) -> Result<Vec<Repli>> {
    let mut trouves = Vec::new();
    for code in paths.classeurs_existants() {
        let conn = ygo_db::connexion::ouvrir_lecture_seule(paths.classeur_db(&code))?;
        let lignes: Vec<(String, Option<i64>)> = {
            let mut r = conn.prepare(
                "SELECT DISTINCT card_image_url, card_image_id FROM cards
                  WHERE card_image_url LIKE '%yugipedia.com%'",
            )?;
            let v = r
                .query_map([], |l| Ok((l.get(0)?, l.get(1)?)))?
                .collect::<std::result::Result<_, _>>()?;
            v
        };
        for (url, id) in lignes {
            let Some(nom) = ygo_images::plan::nom_de_fichier(&url) else {
                continue;
            };
            if !nom.to_ascii_lowercase().ends_with(".png") {
                continue;
            }
            let destination = match id {
                Some(i) if i != 0 => paths.img_small().join(&nom),
                _ => paths.img().join(&code).join(&nom),
            };
            if destination.is_file() && est_jpeg(&destination) {
                trouves.push(Repli {
                    destination: relatif(paths, &destination),
                    url,
                });
            }
        }
    }
    trouves.sort();
    trouves.dedup();
    Ok(trouves)
}

/// Ce que la reprise a fait.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Reprise {
    /// Classeurs dont les images de tirage ont été reposées.
    pub classeurs: usize,
    /// Lignes qui ont reçu une adresse d'image de tirage nouvelle.
    pub adresses_reposees: usize,
    /// Replis connus (registre et détection confondus).
    pub replis: usize,
    /// Requêtes de vérification envoyées à Yugipedia (50 fichiers chacune).
    pub verifications: usize,
    /// Fichiers que Yugipedia a désormais : leur repli est rouvert.
    pub presents: usize,
    /// Fichiers toujours absents : leur repli est gardé, et revérifié à la
    /// mise à jour suivante.
    pub absents: usize,
    /// Fichiers de repli effacés, à retélécharger à la prochaine ouverture.
    pub effaces: usize,
}

/// Les adresses d'image Yugipedia que les classeurs portent encore.
fn adresses_referencees(paths: &Paths) -> HashSet<String> {
    let mut toutes = HashSet::new();
    for code in paths.classeurs_existants() {
        let Ok(conn) = ygo_db::connexion::ouvrir_lecture_seule(paths.classeur_db(&code)) else {
            continue;
        };
        let Ok(mut r) = conn.prepare(
            "SELECT DISTINCT card_image_url FROM cards WHERE card_image_url LIKE '%yugipedia.com%'",
        ) else {
            continue;
        };
        let lues: Vec<String> = r
            .query_map([], |l| l.get::<_, String>(0))
            .map(|l| l.filter_map(std::result::Result::ok).collect())
            .unwrap_or_default();
        toutes.extend(lues);
    }
    toutes
}

/// Les replis connus : le registre — moins les entrées dont plus aucun
/// classeur ne porte l'adresse — et la détection.
#[must_use]
pub fn connus(paths: &Paths) -> Vec<Repli> {
    let referencees = adresses_referencees(paths);
    let mut replis: Vec<Repli> = lire(paths)
        .into_iter()
        .filter(|r| !ygo_sources::http::est_yugipedia(&r.url) || referencees.contains(&r.url))
        .collect();
    match detecter(paths) {
        Ok(d) => replis.extend(d),
        Err(e) => tracing::warn!(erreur = %e, "détection des replis incomplète"),
    }
    replis.sort();
    replis.dedup_by(|a, b| a.destination == b.destination);
    replis
}

/// Les noms de fichiers Yugipedia à vérifier, sans doublon.
#[must_use]
pub fn noms_a_verifier(replis: &[Repli]) -> Vec<String> {
    let mut noms: Vec<String> = replis
        .iter()
        .filter(|r| ygo_sources::http::est_yugipedia(&r.url))
        .filter_map(|r| ygo_sources::yugipedia::nom_de_fichier(&r.url))
        .collect();
    noms.sort();
    noms.dedup();
    noms
}

/// Rouvre les replis dont la source primaire a désormais le fichier : image
/// de repli effacée, 404 oublié. Les autres restent tels quels et au registre.
///
/// Un repli d'une autre source que Yugipedia n'a pas de vérification groupée
/// possible : il est rouvert comme avant.
///
/// # Errors
///
/// Rend une erreur si le registre ne peut pas être réécrit.
pub fn rouvrir(
    paths: &Paths,
    replis: &[Repli],
    presents: &HashSet<String>,
    bilan: &mut Reprise,
) -> Result<()> {
    let cache = ygo_sources::cache::actif();
    let mut gardes = Vec::new();
    for r in replis {
        let yugipedia = ygo_sources::http::est_yugipedia(&r.url);
        let present =
            ygo_sources::yugipedia::nom_de_fichier(&r.url).is_some_and(|n| presents.contains(&n));
        if yugipedia && !present {
            bilan.absents += 1;
            gardes.push(r.clone());
            continue;
        }
        if yugipedia {
            bilan.presents += 1;
        }
        let fichier = absolu(paths, &r.destination);
        if fichier.is_file() {
            match std::fs::remove_file(&fichier) {
                Ok(()) => bilan.effaces += 1,
                Err(e) => {
                    tracing::warn!(fichier = %fichier.display(), erreur = %e, "repli non effacé");
                    gardes.push(r.clone());
                    continue;
                }
            }
        }
        if let Some(c) = cache {
            c.oublier_introuvable(&r.url);
        }
    }
    ecrire(paths, &gardes)
}

/// Repose les adresses d'image de tirage de chaque classeur depuis la base.
fn reposer_adresses(paths: &Paths, bilan: &mut Reprise) {
    let reference = Priorites::charger(paths.rarity_config());
    for code in paths.classeurs_existants() {
        match crate::images_tirage::poser(
            &paths.classeur_db(&code),
            &paths.cardinfo_db(),
            &reference,
        ) {
            Ok(r) => {
                bilan.classeurs += 1;
                bilan.adresses_reposees += r.a_poser.len();
            }
            Err(e) => {
                tracing::warn!(classeur = %code, erreur = %e, "images de tirage non reposées")
            }
        }
    }
}

/// La reprise d'après mise à jour.
///
/// 1. Reposer les images de tirage depuis la nouvelle base.
/// 2. Demander à Yugipedia, **par lots de 50 en une requête**, lesquels des
///    fichiers manquants il a désormais ([`ygo_sources::yugipedia::fichiers_existants`]).
/// 3. Ne rouvrir que ceux-là : leur vraie image est téléchargée à la
///    prochaine ouverture du classeur, **une fois** — après quoi ce n'est
///    plus un repli, et plus rien ne la redemande.
///
/// Sans réponse de Yugipedia (hors ligne, panne), rien n'est rouvert : les
/// replis restent au registre pour la mise à jour suivante.
///
/// # Errors
///
/// Rend une erreur si le registre ne peut pas être réécrit. Un classeur
/// illisible est journalisé et sauté.
pub async fn reprendre(paths: &Paths, client: &ygo_sources::ClientHttp) -> Result<Reprise> {
    let mut bilan = Reprise::default();
    reposer_adresses(paths, &mut bilan);

    let replis = connus(paths);
    bilan.replis = replis.len();
    if replis.is_empty() {
        ecrire(paths, &[])?;
        return Ok(bilan);
    }
    let noms = noms_a_verifier(&replis);
    bilan.verifications = noms
        .len()
        .div_ceil(ygo_sources::yugipedia::TITRES_PAR_REQUETE);
    let presents = if noms.is_empty() {
        HashSet::new()
    } else {
        match ygo_sources::yugipedia::fichiers_existants(client, &noms).await {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(erreur = %e, "Yugipedia injoignable — replis gardés pour la prochaine fois");
                bilan.absents = replis.len();
                ecrire(paths, &replis)?;
                return Ok(bilan);
            }
        }
    };
    rouvrir(paths, &replis, &presents, &mut bilan)?;
    Ok(bilan)
}

/// Lit le `rowid` → adresse d'un classeur — pour les tests.
#[cfg(test)]
fn adresses(chemin: &Path) -> Vec<(i64, String)> {
    let conn = rusqlite::Connection::open(chemin).unwrap_or_else(|_| unreachable!());
    let mut r = conn
        .prepare("SELECT rowid, card_image_url FROM cards ORDER BY rowid")
        .unwrap_or_else(|_| unreachable!());
    r.query_map([], |l| Ok((l.get(0)?, l.get(1)?)))
        .unwrap_or_else(|_| unreachable!())
        .filter_map(std::result::Result::ok)
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;
    use rusqlite::Connection;

    const URL_YP: &str = "https://ms.yugipedia.com//9/92/TheFluteofGuidingDragon-LOCR-JP-UR.png";

    fn installation() -> (tempfile::TempDir, Paths) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(tmp.path());
        let chemin = paths.classeur_db("LOCR-JP");
        std::fs::create_dir_all(chemin.parent().unwrap()).unwrap();
        let conn = Connection::open(&chemin).unwrap();
        conn.execute_batch(ygo_db::schema::DDL_CLASSEUR).unwrap();
        conn.execute(
            "INSERT INTO cards (name, set_code, rarity, card_image_url, card_image_id)
             VALUES ('The Flute of Guiding Dragon', 'LOCR-JP003', 'Ultra Rare', ?1, 43973174)",
            [URL_YP],
        )
        .unwrap();
        std::fs::create_dir_all(paths.img_small()).unwrap();
        (tmp, paths)
    }

    #[test]
    fn le_registre_garde_chaque_destination_une_fois() {
        let (_tmp, paths) = installation();
        let dest = paths.img_small().join("A.png");
        consigner(&paths, &[(dest.clone(), "u1".into())]);
        consigner(&paths, &[(dest, "u2".into())]);
        assert_eq!(
            lire(&paths),
            vec![Repli {
                destination: "small/A.png".into(),
                url: "u2".into()
            }]
        );
        consigner(&paths, &[]);
        assert_eq!(lire(&paths).len(), 1, "rien à consigner : rien ne change");
    }

    /// La signature des 27 de LOCR-JP : un `.png` Yugipedia qui contient un
    /// JPEG.
    #[test]
    fn un_png_yugipedia_qui_contient_un_jpeg_est_un_repli() {
        let (_tmp, paths) = installation();
        let fichier = paths
            .img_small()
            .join("TheFluteofGuidingDragon-LOCR-JP-UR.png");
        std::fs::write(&fichier, [0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3]).unwrap();
        assert_eq!(detecter(&paths).unwrap().len(), 1);
        // Un vrai PNG n'en est pas un.
        std::fs::write(&fichier, b"\x89PNG\r\n\x1a\n...").unwrap();
        assert!(detecter(&paths).unwrap().is_empty());
    }

    /// Seul le fichier que Yugipedia a désormais est rouvert ; l'absent garde
    /// son image de repli et reste au registre pour la mise à jour suivante.
    #[test]
    fn seul_ce_que_yugipedia_a_desormais_est_rouvert() {
        let (_tmp, paths) = installation();
        let conn = Connection::open(paths.classeur_db("LOCR-JP")).unwrap();
        conn.execute(
            "INSERT INTO cards (name, set_code, rarity, card_image_url, card_image_id)
             VALUES ('X', 'LOCR-JP004', 'Ultra Rare', 'https://ms.yugipedia.com//a/ab/Arrive.png', 7)",
            [],
        )
        .unwrap();
        drop(conn);
        let absent = paths
            .img_small()
            .join("TheFluteofGuidingDragon-LOCR-JP-UR.png");
        std::fs::write(&absent, [0xFF, 0xD8, 0xFF, 0xE0]).unwrap();
        let arrive = paths.img_small().join("Arrive.png");
        std::fs::write(&arrive, [0xFF, 0xD8, 0xFF, 0xE0]).unwrap();

        let replis = connus(&paths);
        assert_eq!(replis.len(), 2);
        assert_eq!(
            noms_a_verifier(&replis),
            vec![
                "Arrive.png".to_owned(),
                "TheFluteofGuidingDragon-LOCR-JP-UR.png".to_owned()
            ]
        );
        // Réponse de Yugipedia : seul « Arrive.png » existe désormais.
        let presents: HashSet<String> = ["Arrive.png".to_owned()].into();
        let mut bilan = Reprise::default();
        rouvrir(&paths, &replis, &presents, &mut bilan).unwrap();
        assert_eq!((bilan.presents, bilan.absents, bilan.effaces), (1, 1, 1));
        assert!(
            !arrive.exists(),
            "rouvert : la vraie image viendra à l'ouverture"
        );
        assert!(absent.exists(), "gardé : l'image de repli reste affichée");
        assert_eq!(lire(&paths).len(), 1, "l'absent reste au registre");
        // Le classeur n'a pas bougé.
        assert_eq!(
            adresses(&paths.classeur_db("LOCR-JP"))[0],
            (1, URL_YP.to_owned())
        );
    }

    /// Une entrée du registre dont plus aucun classeur ne porte l'adresse
    /// est oubliée.
    #[test]
    fn le_registre_oublie_les_adresses_disparues() {
        let (_tmp, paths) = installation();
        consigner(
            &paths,
            &[(
                paths.img_small().join("Vieux.png"),
                "https://ms.yugipedia.com//a/ab/Vieux.png".into(),
            )],
        );
        assert_eq!(lire(&paths).len(), 1);
        assert!(connus(&paths).is_empty());
    }
}
