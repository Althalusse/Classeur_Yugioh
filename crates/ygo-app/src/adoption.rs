// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Reprendre une installation existante dans une installation neuve.
//!
//! # Pourquoi ceci existe
//!
//! Le portage a vécu **dans** le dossier de la V1.0.4 : `Lancer-Appli.ps1`
//! passe `Projet Python\V1.0.4` en racine, et les deux applications partagent
//! `bdd/`, `img/` et `logs/`. C'était commode — les mêmes vignettes, les mêmes
//! classeurs, aucune conversion — et le §3.3 du cahier en fait même un
//! invariant : « la V2 doit ouvrir une installation V1.0.4 sans conversion ».
//!
//! Cette cohabitation a un prix, constaté le 2026-09-05 : une reconstruction
//! de base par le Rust a effacé les 1 613 lignes de `card_images_externes` que
//! le Python avait produites. Deux applications, un seul jeu de fichiers.
//!
//! Ce module fait le pas d'après : donner au Rust **sa** racine, en y
//! apportant ce qui compte.
//!
//! # Ce qui est repris, et ce qui ne l'est pas
//!
//! | | | |
//! |---|---|---|
//! | classeurs (`bdd/classeur_creer/`) | **repris** | irremplaçable — c'est la collection |
//! | corbeille (`bdd/corbeille/`) | **repris** | des classeurs supprimés qu'on peut vouloir rendre |
//! | `app_config.json`, `rarity_config.json` | **repris** | des réglages, pas des données à refaire |
//! | `img/` | **repris** | reconstructible, mais des milliers de requêtes |
//! | `export/` | **repris** | les CSV que l'utilisateur a produits |
//! | `cardinfo.db` | **non** | dix secondes de réseau la refont, et 163 Mo de disque en jeu |
//! | `last_update.txt` | **non** | sans base, une version serait un mensonge |
//! | `fr_names_cache.json` | **non** | jamais lu par ce portage |
//! | `card_images_externes`, `logs/` | **non** | à la V1.0.4 ; les prendre serait les lui reprendre |
//!
//! `cardinfo.db` non reprise est le choix qui structure le reste : il fait
//! tomber le coût de 450 à 290 Mo, et une base reconstruite est plus sûre
//! qu'une base copiée — celle-ci sera la première jamais construite par le
//! Rust pour lui-même.
//!
//! # Copier, pas déplacer
//!
//! La source n'est **jamais** touchée. La V1.0.4 reste entière et utilisable :
//! elle est l'oracle de non-régression du portage, et le restera jusqu'à la
//! bascule. Une adoption qui déplacerait les fichiers ferait disparaître le
//! seul point de comparaison du projet.

use std::path::{Path, PathBuf};

use ygo_core::paths::Paths;

use crate::error::{AppError, Result};

/// Ce qu'une adoption a repris.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bilan {
    /// Classeurs repris, par code.
    pub classeurs: Vec<String>,
    /// Entrées de corbeille reprises.
    pub rebuts: usize,
    /// Fichiers de réglage repris.
    pub reglages: Vec<String>,
    /// Images reprises.
    pub images: usize,
    /// Octets copiés, tous fichiers confondus.
    pub octets: u64,
    /// Fichiers d'export repris.
    pub exports: usize,
}

impl Bilan {
    /// Les mégaoctets copiés, pour l'affichage.
    #[must_use]
    pub fn mo(&self) -> u64 {
        self.octets / 1_048_576
    }
}

/// Reprend une installation dans une racine neuve.
///
/// La destination doit être vide ou inexistante : adopter par-dessus une
/// installation qui vit déjà mélangerait deux collections, et le résultat
/// n'aurait de sens pour personne.
///
/// # Errors
///
/// - la source n'est pas une installation (pas de `bdd/`) ;
/// - la destination existe et contient déjà des données ;
/// - une copie échoue.
pub fn adopter(source: &Path, destination: &Path) -> Result<Bilan> {
    let origine = Paths::depuis_racine(source);
    let cible = Paths::depuis_racine(destination);

    if !origine.bdd().is_dir() {
        return Err(AppError::Creation(format!(
            "{} n'est pas une installation : aucun dossier `bdd/`.",
            source.display()
        )));
    }
    if cible.bdd().exists() || cible.classeurs().exists() {
        return Err(AppError::Creation(format!(
            "{} contient déjà une installation. Choisissez une destination vide.",
            destination.display()
        )));
    }
    if source == destination {
        return Err(AppError::Creation(
            "la source et la destination sont le même dossier".to_owned(),
        ));
    }

    cible.creer_dossiers()?;
    let mut bilan = Bilan::default();

    // Les classeurs d'abord : c'est ce qui ne se refait pas.
    for code in origine.classeurs_existants() {
        let de = origine.dossier_classeur(&code);
        let vers = cible.dossier_classeur(&code);
        bilan.octets += copier_dossier(&de, &vers)?;
        bilan.classeurs.push(code);
    }
    bilan.classeurs.sort();

    // La corbeille : des classeurs supprimés qu'on peut vouloir rendre.
    let corbeille_source = crate::suppression::corbeille(&origine);
    if corbeille_source.is_dir() {
        let corbeille_cible = crate::suppression::corbeille(&cible);
        bilan.octets += copier_dossier(&corbeille_source, &corbeille_cible)?;
        bilan.rebuts = entrees(&corbeille_cible);
    }

    // Les réglages — pas les caches, pas la version.
    for (de, vers, nom) in [
        (origine.app_config(), cible.app_config(), "app_config.json"),
        (
            origine.rarity_config(),
            cible.rarity_config(),
            "rarity_config.json",
        ),
    ] {
        if de.is_file() {
            bilan.octets += copier_fichier(&de, &vers)?;
            bilan.reglages.push(nom.to_owned());
        }
    }

    // Les images : reconstructibles, mais au prix de milliers de requêtes.
    if origine.img().is_dir() {
        bilan.octets += copier_dossier(&origine.img(), &cible.img())?;
        bilan.images = fichiers_recursifs(&cible.img());
    }

    // Les exports : ce que l'utilisateur a produit, et que rien ne refait.
    if origine.export().is_dir() {
        bilan.octets += copier_dossier(&origine.export(), &cible.export())?;
        bilan.exports = entrees(&cible.export());
    }

    tracing::info!(
        source = %source.display(),
        destination = %destination.display(),
        classeurs = bilan.classeurs.len(),
        images = bilan.images,
        mo = bilan.mo(),
        "installation adoptée"
    );
    Ok(bilan)
}

/// Copie un dossier entier, récursivement. Rend les octets copiés.
fn copier_dossier(de: &Path, vers: &Path) -> Result<u64> {
    std::fs::create_dir_all(vers)?;
    let mut octets = 0;
    for entree in std::fs::read_dir(de)? {
        let entree = entree?;
        let chemin = entree.path();
        let destination = vers.join(entree.file_name());
        if entree.file_type()?.is_dir() {
            octets += copier_dossier(&chemin, &destination)?;
        } else if entree.file_type()?.is_file() {
            // Les fichiers annexes de SQLite ne se copient pas : un `-wal`
            // sans sa base, ou pire avec une base copiée à un autre instant,
            // ferait rejouer un journal qui ne lui correspond pas.
            let nom = entree.file_name();
            let nom = nom.to_string_lossy();
            if nom.ends_with("-wal") || nom.ends_with("-shm") {
                continue;
            }
            octets += copier_fichier(&chemin, &destination)?;
        }
    }
    Ok(octets)
}

/// Copie un fichier, en créant son dossier. Rend les octets copiés.
fn copier_fichier(de: &Path, vers: &Path) -> Result<u64> {
    if let Some(parent) = vers.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(std::fs::copy(de, vers)?)
}

/// Nombre d'entrées directes d'un dossier.
fn entrees(dossier: &Path) -> usize {
    std::fs::read_dir(dossier).map_or(0, |lecture| lecture.flatten().count())
}

/// Nombre de fichiers d'un dossier, sous-dossiers compris.
fn fichiers_recursifs(dossier: &Path) -> usize {
    let Ok(lecture) = std::fs::read_dir(dossier) else {
        return 0;
    };
    let mut total = 0;
    for entree in lecture.flatten() {
        let Ok(genre) = entree.file_type() else {
            continue;
        };
        if genre.is_dir() {
            total += fichiers_recursifs(&entree.path());
        } else if genre.is_file() {
            total += 1;
        }
    }
    total
}

/// Les chemins d'une installation qu'une adoption laisse **délibérément**.
///
/// Exposé pour que l'interface et le `ygo-cli` puissent le dire sans le
/// recopier — et pour qu'un test constate que la liste n'a pas changé en
/// silence.
#[must_use]
pub fn non_repris() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "bdd/cardinfo.db",
            "reconstruite en dix secondes — 163 Mo de disque épargnés",
        ),
        (
            "bdd/last_update.txt",
            "sans base, une version serait un mensonge",
        ),
        (
            "bdd/fr_names_cache.json",
            "jamais lu par ce portage — les noms FR viennent de YGOJSON",
        ),
        (
            "bdd/card_images_externes",
            "table de la V1.0.4, que ce portage ne lit pas",
        ),
        ("logs/", "le journal de la V1.0.4 lui appartient"),
    ]
}

/// Les dossiers d'une installation, pour un affichage avant adoption.
///
/// Rend `(nom, octets)` pour ce qui sera copié. Permet d'annoncer le coût
/// avant de l'engager — sur un disque presque plein, ce n'est pas un détail.
#[must_use]
pub fn a_reprendre(source: &Path) -> Vec<(String, u64)> {
    let origine = Paths::depuis_racine(source);
    let mut mesures = Vec::new();
    for (nom, chemin) in [
        ("classeurs".to_owned(), origine.classeurs()),
        (
            "corbeille".to_owned(),
            crate::suppression::corbeille(&origine),
        ),
        ("images".to_owned(), origine.img()),
        ("export".to_owned(), origine.export()),
    ] {
        let taille = taille_dossier(&chemin);
        if taille > 0 {
            mesures.push((nom, taille));
        }
    }
    mesures
}

/// Somme des tailles d'un dossier, récursivement.
fn taille_dossier(dossier: &Path) -> u64 {
    let Ok(lecture) = std::fs::read_dir(dossier) else {
        return 0;
    };
    let mut total = 0;
    for entree in lecture.flatten() {
        let Ok(genre) = entree.file_type() else {
            continue;
        };
        if genre.is_dir() {
            total += taille_dossier(&entree.path());
        } else if let Ok(meta) = entree.metadata() {
            total += meta.len();
        }
    }
    total
}

/// Le chemin de destination d'une [`adopter`] à venir, pour un dossier donné.
type Chemins = (PathBuf, PathBuf);

/// Sépare la source et la destination d'une ligne de commande.
///
/// # Errors
///
/// Rend une erreur si l'un des deux manque.
pub fn arguments(args: &[String]) -> Result<Chemins> {
    let Some(source) = args.first() else {
        return Err(AppError::Creation(
            "chemin de l'installation source attendu".to_owned(),
        ));
    };
    let Some(destination) = args.get(1) else {
        return Err(AppError::Creation(
            "chemin de la destination attendu".to_owned(),
        ));
    };
    Ok((PathBuf::from(source), PathBuf::from(destination)))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    /// Fabrique une installation de test peuplée.
    fn installation(racine: &Path) -> Paths {
        let paths = Paths::depuis_racine(racine);
        paths.creer_dossiers().unwrap();

        for code in ["RA02", "LOCR-JP"] {
            std::fs::create_dir_all(paths.dossier_classeur(code)).unwrap();
            std::fs::write(paths.classeur_db(code), format!("base de {code}")).unwrap();
            // Les fichiers annexes de SQLite : présents, jamais copiés.
            std::fs::write(
                format!("{}-wal", paths.classeur_db(code).display()),
                b"journal",
            )
            .unwrap();
        }
        std::fs::write(paths.app_config(), b"{}").unwrap();
        std::fs::write(paths.rarity_config(), b"{\"Common\":1}").unwrap();
        std::fs::write(paths.cardinfo_db(), vec![0_u8; 4096]).unwrap();
        std::fs::write(paths.last_update(), b"[]").unwrap();
        std::fs::write(paths.fr_names_cache(), b"{}").unwrap();

        std::fs::create_dir_all(paths.img_small()).unwrap();
        std::fs::write(paths.img_small().join("1.jpg"), b"image").unwrap();
        std::fs::write(paths.img_small().join("2.jpg"), b"image").unwrap();
        std::fs::create_dir_all(paths.img_boosters()).unwrap();
        std::fs::write(paths.img_boosters().join("RA02.png"), b"cover").unwrap();

        std::fs::create_dir_all(paths.export()).unwrap();
        std::fs::write(paths.export().join("collection.csv"), b"Code,Rarete").unwrap();
        paths
    }

    #[test]
    fn une_adoption_reprend_la_collection_et_les_images() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("v104");
        let destination = tmp.path().join("rust");
        installation(&source);

        let bilan = adopter(&source, &destination).unwrap();
        assert_eq!(bilan.classeurs, vec!["LOCR-JP", "RA02"]);
        assert_eq!(bilan.images, 3, "deux vignettes et une couverture");
        assert_eq!(bilan.exports, 1);
        assert_eq!(
            bilan.reglages,
            vec!["app_config.json", "rarity_config.json"]
        );

        let cible = Paths::depuis_racine(&destination);
        assert_eq!(
            std::fs::read_to_string(cible.classeur_db("RA02")).unwrap(),
            "base de RA02"
        );
        assert!(cible.img_boosters().join("RA02.png").is_file());
    }

    /// La base n'est pas copiée : elle se reconstruit, et elle pèse 163 Mo.
    #[test]
    fn la_base_de_reference_n_est_pas_reprise() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("v104");
        let destination = tmp.path().join("rust");
        installation(&source);

        adopter(&source, &destination).unwrap();
        let cible = Paths::depuis_racine(&destination);
        assert!(!cible.cardinfo_db().exists(), "reconstruite, pas copiée");
        assert!(
            !cible.last_update().exists(),
            "et sans base, aucune version à déclarer"
        );
        assert!(!cible.fr_names_cache().exists(), "jamais lu par ce portage");
    }

    /// La source n'est jamais touchée : la V1.0.4 reste l'oracle.
    #[test]
    fn la_source_reste_intacte() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("v104");
        let destination = tmp.path().join("rust");
        let origine = installation(&source);

        adopter(&source, &destination).unwrap();

        assert!(origine.cardinfo_db().is_file());
        assert!(origine.classeur_db("RA02").is_file());
        assert!(origine.img_small().join("1.jpg").is_file());
        assert!(origine.fr_names_cache().is_file());
    }

    /// Un `-wal` copié à côté d'une base prise à un autre instant ferait
    /// rejouer un journal qui ne lui correspond pas.
    #[test]
    fn les_fichiers_annexes_de_sqlite_ne_suivent_pas() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("v104");
        let destination = tmp.path().join("rust");
        installation(&source);

        adopter(&source, &destination).unwrap();
        let cible = Paths::depuis_racine(&destination);
        assert!(cible.classeur_db("RA02").is_file());
        assert!(
            !PathBuf::from(format!("{}-wal", cible.classeur_db("RA02").display())).exists(),
            "le journal reste où il est"
        );
    }

    /// Adopter par-dessus une installation vivante mélangerait deux
    /// collections : on refuse, plutôt que de produire un résultat que
    /// personne ne saurait démêler.
    #[test]
    fn une_destination_deja_peuplee_est_refusee() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("v104");
        let destination = tmp.path().join("rust");
        installation(&source);
        installation(&destination);

        let erreur = adopter(&source, &destination).unwrap_err();
        assert!(
            erreur.to_string().contains("déjà une installation"),
            "{erreur}"
        );
    }

    #[test]
    fn une_source_qui_n_est_pas_une_installation_est_refusee() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("rien");
        std::fs::create_dir_all(&source).unwrap();
        let erreur = adopter(&source, &tmp.path().join("rust")).unwrap_err();
        assert!(
            erreur.to_string().contains("pas une installation"),
            "{erreur}"
        );
    }

    /// Le coût s'annonce avant d'être engagé — sur un disque presque plein,
    /// ce n'est pas un détail.
    #[test]
    fn le_cout_se_mesure_avant_la_copie() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("v104");
        installation(&source);

        let mesures = a_reprendre(&source);
        let noms: Vec<&str> = mesures.iter().map(|(n, _)| n.as_str()).collect();
        assert!(noms.contains(&"classeurs"), "{noms:?}");
        assert!(noms.contains(&"images"), "{noms:?}");
        assert!(mesures.iter().all(|(_, taille)| *taille > 0));
    }

    #[test]
    fn les_arguments_exigent_les_deux_chemins() {
        assert!(arguments(&[]).is_err());
        assert!(arguments(&["source".to_owned()]).is_err());
        let (de, vers) = arguments(&["a".to_owned(), "b".to_owned()]).unwrap();
        assert_eq!(de, PathBuf::from("a"));
        assert_eq!(vers, PathBuf::from("b"));
    }

    /// La liste des non-repris est documentaire : qu'elle change se voit.
    #[test]
    fn la_liste_des_non_repris_est_explicite() {
        let liste = non_repris();
        assert_eq!(liste.len(), 5);
        assert!(liste.iter().all(|(_, raison)| !raison.is_empty()));
        assert!(liste.iter().any(|(quoi, _)| *quoi == "bdd/cardinfo.db"));
    }
}
