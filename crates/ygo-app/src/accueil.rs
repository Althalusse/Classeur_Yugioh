// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Les données de l'écran d'accueil — la liste des classeurs.
//!
//! Portage de `ecran_accueil._load_data` et de `_find_cover`, **sans une seule
//! ligne d'interface**.
//!
//! # Pourquoi ce module existe séparément
//!
//! C'est le premier lot dont une moitié ne pourra jamais être prouvée : il
//! n'existe pas d'oracle pour « est-ce que l'écran a la bonne tête ». La
//! parade est de faire passer la frontière au bon endroit. Tout ce que
//! l'accueil **affiche** est calculé ici, et se compare champ par champ à ce
//! que le Python produit sur les 26 classeurs réels des deux installations.
//! Ne restent à l'interface que les pixels.
//!
//! C'est aussi ce qui permet d'écrire l'écran **deux fois** — une fois par
//! framework candidat — sans dupliquer une seule règle métier.

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use ygo_core::paths::Paths;

use crate::error::Result;

/// Un classeur, tel que l'accueil le montre.
#[derive(Debug, Clone, PartialEq)]
pub struct Classeur {
    /// Code du set, qui est aussi le nom du dossier.
    pub code: String,
    /// Nom complet du set, en anglais.
    pub nom: String,
    /// Nom complet en français, à défaut le nom anglais.
    pub nom_fr: String,
    /// Nombre de cartes du classeur.
    pub total: usize,
    /// Nombre de cartes possédées.
    pub possedees: usize,
    /// Colonnes de la grille d'affichage.
    pub colonnes: u8,
    /// Lignes de la grille d'affichage.
    pub lignes: u8,
    /// `card_image_id` d'une carte représentative, pour la couverture.
    pub image_id: Option<String>,
    /// Image de couverture, si l'une des quatre pistes a abouti.
    pub couverture: Option<PathBuf>,
    /// Laquelle des quatre pistes a abouti.
    pub origine_couverture: OrigineCouverture,
}

impl Classeur {
    /// Part de cartes possédées, en pourcentage.
    ///
    /// Le Python la calcule dans `get_stats_collection` ; elle est dérivée
    /// plutôt que stockée, pour qu'elle ne puisse pas contredire ses deux
    /// termes.
    #[must_use]
    pub fn pourcentage(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        #[allow(clippy::cast_precision_loss)]
        {
            self.possedees as f64 / self.total as f64 * 100.0
        }
    }

    /// Le nom à afficher selon la langue.
    #[must_use]
    pub fn nom_affiche(&self, francais: bool) -> &str {
        if francais {
            &self.nom_fr
        } else {
            &self.nom
        }
    }
}

/// D'où vient l'image de couverture d'un classeur.
///
/// Portage des quatre niveaux de `_find_cover`, dans l'ordre. Le niveau
/// retenu est conservé : c'est ce qui permet de dire, à la relecture d'une
/// capture, pourquoi tel classeur n'a pas d'image plutôt que de le constater.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrigineCouverture {
    /// `img/boosters/<CODE>.<ext>` — l'image officielle du booster.
    Booster,
    /// `img/small/<card_image_id>.jpg` — une carte du classeur.
    Carte,
    /// `img/<CODE>/*.jpg` — ancien dossier par classeur (YGOJSON historique).
    Dossier,
    /// Aucune des trois : l'interface montrera un substitut.
    Aucune,
}

impl OrigineCouverture {
    /// Le nom que l'oracle Python emploie.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Booster => "booster",
            Self::Carte => "carte",
            Self::Dossier => "dossier",
            Self::Aucune => "aucune",
        }
    }
}

/// Tout ce que l'accueil lit dans la base d'un classeur.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MetaClasseur {
    /// Nombre de cartes.
    pub total: usize,
    /// Nombre de cartes possédées.
    pub possedees: usize,
    /// Colonnes de la grille.
    pub colonnes: u8,
    /// Lignes de la grille.
    pub lignes: u8,
    /// `card_image_id` d'une carte représentative.
    pub image_id: Option<String>,
    /// `set_name`, s'il est renseigné.
    pub set_name: Option<String>,
}

/// Les métadonnées d'un classeur, lues en **une seule** ouverture SQLite.
///
/// Portage de `get_classeur_meta_full`, élargi à ce que l'accueil consomme.
///
/// # Pourquoi tout est lu d'un coup
///
/// Le Python avait fait ce regroupement en optimisation, et l'avait commenté :
/// « count + grille + image_id en UNE seule ouverture (au lieu de deux par
/// classeur) ». Le portage l'avait **défait** — cinq fonctions propres, chacune
/// ouvrant sa connexion, soit quatre ouvertures par classeur. Mesuré sur
/// l'installation réelle de dix-sept classeurs : **476 ms** pour l'écran
/// d'ouverture de l'application. Le découpage était bon pour les tests et
/// mauvais pour l'utilisateur.
///
/// La leçon mérite d'être écrite : décomposer en petites fonctions pures reste
/// juste, mais **l'unité d'ouverture d'une base n'est pas la fonction, c'est la
/// passe**. Les fonctions détaillées restent publiques et testables une à une ;
/// c'est celle-ci que l'accueil appelle.
///
/// Toute erreur rend les valeurs par défaut, comme le Python : une base
/// corrompue ne doit pas empêcher l'accueil de s'afficher.
#[must_use]
pub fn meta_classeur(chemin: &Path, grille_defaut: (u8, u8)) -> MetaClasseur {
    let defaut = MetaClasseur {
        colonnes: grille_defaut.0,
        lignes: grille_defaut.1,
        ..MetaClasseur::default()
    };
    let Ok(conn) = ygo_db::connexion::ouvrir_lecture_seule(chemin) else {
        return defaut;
    };
    lire_meta(&conn, grille_defaut).unwrap_or(defaut)
}

/// Le corps de [`meta_classeur`], séparé pour être jouable sur une connexion.
fn lire_meta(conn: &Connection, grille_defaut: (u8, u8)) -> Result<MetaClasseur> {
    let cartes: usize = conn
        .query_row("SELECT COUNT(*) FROM cards", [], |l| l.get(0))
        .unwrap_or(0);

    let cle = |cle: &str| -> Option<u8> {
        conn.query_row("SELECT value FROM meta WHERE key = ?1", [cle], |l| {
            l.get::<_, String>(0)
        })
        .ok()
        .and_then(|v| v.trim().parse().ok())
    };
    let colonnes = cle("colonnes").unwrap_or(grille_defaut.0);
    let lignes = cle("lignes").unwrap_or(grille_defaut.1);

    let image_id: Option<String> = conn
        .query_row(
            "SELECT card_image_id FROM cards WHERE card_image_id IS NOT NULL LIMIT 1",
            [],
            |l| l.get::<_, rusqlite::types::Value>(0),
        )
        .ok()
        .and_then(|v| match v {
            rusqlite::types::Value::Integer(i) => Some(i.to_string()),
            rusqlite::types::Value::Text(t) => Some(t),
            _ => None,
        });

    let possedees: usize = conn
        .query_row("SELECT COUNT(*) FROM cards WHERE possessed = 1", [], |l| {
            l.get(0)
        })
        .unwrap_or(0);

    let set_name: Option<String> = conn
        .query_row(
            "SELECT set_name FROM cards WHERE set_name IS NOT NULL AND set_name != '' LIMIT 1",
            [],
            |l| l.get(0),
        )
        .ok();

    Ok(MetaClasseur {
        total: cartes,
        possedees,
        colonnes,
        lignes,
        image_id,
        set_name,
    })
}

/// Le nombre de cartes possédées d'un classeur, en ouvrant sa base.
///
/// Portage de la part de `get_stats_collection` que l'accueil consomme. Le
/// Python calcule aussi la répartition par rareté ; elle appartient à l'écran
/// des statistiques, pas ici.
///
/// [`lister`] ne l'appelle **pas** : elle lit ce compte dans la même ouverture
/// que le reste. Cette fonction sert à interroger un classeur isolé.
#[must_use]
pub fn possedees(chemin: &Path) -> usize {
    let Ok(conn) = ygo_db::connexion::ouvrir_lecture_seule(chemin) else {
        return 0;
    };
    conn.query_row("SELECT COUNT(*) FROM cards WHERE possessed = 1", [], |l| {
        l.get(0)
    })
    .unwrap_or(0)
}

/// Le nom complet d'un set, en partant du `set_name` déjà lu.
///
/// Portage de `get_set_title`. Le classeur fait autorité — il porte son propre
/// `set_name` depuis la migration — et `cardinfo.db` ne sert que de repli pour
/// les classeurs antérieurs.
///
/// `set_name` est passé en paramètre plutôt que relu : la base du classeur a
/// déjà été ouverte par [`meta_classeur`], et la rouvrir pour un seul champ
/// était précisément le gaspillage que ce module a corrigé.
///
/// # Le `OR` qui couvre deux natures de préfixe
///
/// Un classeur TCG (`CROS`) a des locales `CROS-EN`, `CROS-EU`… — d'où le
/// `LIKE 'CROS-%'`. Un classeur OCG (`LOCR-JP`) porte **déjà** son suffixe de
/// langue, et sa locale est exactement `LOCR-JP` — d'où l'égalité. Les deux
/// dans une seule requête, comme le Python.
///
/// # Écart reproduit
///
/// Quand le classeur porte un `set_name`, `francais` **n'a aucun effet** :
/// c'est le comportement du Python, et il fait que l'interface française
/// affiche le nom anglais de tout set migré. Bug latent consigné, non corrigé
/// ici (règle R9).
#[must_use]
pub fn titre_set(
    cardinfo: Option<&Connection>,
    code_set: &str,
    set_name: Option<&str>,
    francais: bool,
) -> Option<String> {
    if let Some(nom) = set_name.filter(|n| !n.is_empty()) {
        return Some(nom.to_owned());
    }
    let conn = cardinfo?;
    let code = code_set.trim().to_uppercase();
    let existe = conn
        .prepare("SELECT 1 FROM sqlite_master WHERE type='table' AND name='sets'")
        .ok()?
        .exists([])
        .ok()?;
    if !existe {
        return None;
    }
    let (fr, en): (Option<String>, Option<String>) = conn
        .query_row(
            "SELECT s.name_fr, s.name_en FROM sets s \
             JOIN set_locales sl ON sl.set_uuid = s.uuid \
             WHERE sl.prefix LIKE ?1 OR UPPER(sl.prefix) = ?2 LIMIT 1",
            (format!("{code}-%"), &code),
            |l| Ok((l.get(0)?, l.get(1)?)),
        )
        .ok()?;
    if francais {
        fr.filter(|t| !t.is_empty()).or(en)
    } else {
        en
    }
    .filter(|t| !t.is_empty())
}

/// Cherche l'image de couverture d'un classeur.
///
/// Portage de `_find_cover`, ses quatre niveaux dans l'ordre :
///
/// 1. l'image officielle du booster, `img/boosters/<CODE>.<ext>` ;
/// 2. une carte du classeur, `img/small/<card_image_id>.jpg` ;
/// 3. l'ancien dossier par classeur, `img/<CODE>/*.jpg`, **premier par ordre
///    alphabétique** ;
/// 4. rien.
///
/// Ne télécharge jamais : le Python lance ses téléchargements de boosters dans
/// un fil séparé, après l'affichage. Cette fonction est celle que l'interface
/// peut appeler sans bloquer.
#[must_use]
pub fn chercher_couverture(
    paths: &Paths,
    code_set: &str,
    image_id: Option<&str>,
) -> (Option<PathBuf>, OrigineCouverture) {
    if let Some(booster) = paths.chercher_cover(code_set) {
        return (Some(booster), OrigineCouverture::Booster);
    }

    if paths.img_small().is_dir() {
        if let Some(id) = image_id {
            let chemin = paths.img_small().join(format!("{id}.jpg"));
            if chemin.is_file() {
                return (Some(chemin), OrigineCouverture::Carte);
            }
        }
    }

    let par_classeur = paths.img().join(code_set);
    if par_classeur.is_dir() {
        if let Ok(entrees) = std::fs::read_dir(&par_classeur) {
            let mut fichiers: Vec<PathBuf> = entrees
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("jpg")))
                .collect();
            fichiers.sort();
            if let Some(premier) = fichiers.first() {
                return (Some(premier.clone()), OrigineCouverture::Dossier);
            }
        }
    }

    (None, OrigineCouverture::Aucune)
}

/// Tous les classeurs d'une installation, triés par code.
///
/// Portage de la boucle de `_load_data`. Le tri est celui de `sorted(os.listdir)`
/// — l'ordre alphabétique du système de fichiers —, et non un ordre métier :
/// l'utilisateur reconnaît ses classeurs à leur place.
///
/// Un dossier sans base rend un classeur à **zéro carte** plutôt que d'être
/// omis : c'est le résidu d'une suppression partielle, et le cacher priverait
/// l'utilisateur du seul endroit d'où il peut le supprimer.
pub fn lister(paths: &Paths, grille_defaut: (u8, u8)) -> Vec<Classeur> {
    let dossier = paths.classeurs();
    let Ok(entrees) = std::fs::read_dir(&dossier) else {
        return Vec::new();
    };
    let mut codes: Vec<String> = entrees
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    codes.sort();

    // `cardinfo.db` n'est ouverte qu'une fois pour toute la liste, et
    // seulement si un classeur en a besoin — c'est-à-dire s'il lui manque son
    // `set_name`. Sur une installation à jour, elle ne s'ouvre jamais.
    let mut cardinfo: Option<Option<Connection>> = None;

    codes
        .into_iter()
        .map(|code| {
            let db = paths.classeur_db(&code);
            let meta = if db.is_file() {
                meta_classeur(&db, grille_defaut)
            } else {
                MetaClasseur {
                    colonnes: grille_defaut.0,
                    lignes: grille_defaut.1,
                    ..MetaClasseur::default()
                }
            };

            let base = if meta.set_name.is_some() {
                None
            } else {
                cardinfo
                    .get_or_insert_with(|| {
                        ygo_db::connexion::ouvrir_lecture_seule(paths.cardinfo_db()).ok()
                    })
                    .as_ref()
            };
            let nom = titre_set(base, &code, meta.set_name.as_deref(), false);
            let nom_fr = titre_set(base, &code, meta.set_name.as_deref(), true);

            let (couverture, origine) = chercher_couverture(paths, &code, meta.image_id.as_deref());
            Classeur {
                nom: nom.unwrap_or_else(|| code.clone()),
                nom_fr: nom_fr.unwrap_or_else(|| code.clone()),
                total: meta.total,
                possedees: meta.possedees,
                colonnes: meta.colonnes,
                lignes: meta.lignes,
                image_id: meta.image_id,
                couverture,
                origine_couverture: origine,
                code,
            }
        })
        .collect()
}

/// Les totaux de la barre du haut : cartes, possédées.
#[must_use]
pub fn totaux(classeurs: &[Classeur]) -> (usize, usize) {
    classeurs
        .iter()
        .fold((0, 0), |(t, p), c| (t + c.total, p + c.possedees))
}

/// Dans quel ordre l'accueil range les classeurs.
///
/// Le Python n'en propose qu'un — l'ordre alphabétique du dossier. Les deux
/// autres sont un ajout, et ils tiennent ici plutôt que dans l'interface parce
/// qu'un ordre est une règle, pas un pixel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Ordre {
    /// Code du set, croissant — l'ordre du Python, et le défaut.
    #[default]
    Code,
    /// Les plus complétés d'abord.
    Completion,
    /// Ceux à qui il manque le plus de cartes d'abord.
    Restantes,
}

impl Ordre {
    /// Le libellé montré dans la liste déroulante.
    #[must_use]
    pub fn libelle(self) -> &'static str {
        match self {
            Self::Code => "Code",
            Self::Completion => "Complétion",
            Self::Restantes => "Cartes manquantes",
        }
    }

    /// Les trois ordres, pour peupler la liste.
    #[must_use]
    pub fn tous() -> [Self; 3] {
        [Self::Code, Self::Completion, Self::Restantes]
    }
}

/// Range les classeurs.
///
/// Le code sert toujours de départage : à complétion ou à manque égal, l'ordre
/// reste stable et prévisible d'un lancement à l'autre. Sans ce second critère,
/// deux classeurs à 100 % changeraient de place au gré du tri.
pub fn trier(classeurs: &mut [Classeur], ordre: Ordre) {
    match ordre {
        Ordre::Code => classeurs.sort_by(|a, b| a.code.cmp(&b.code)),
        Ordre::Completion => classeurs.sort_by(|a, b| {
            b.pourcentage()
                .partial_cmp(&a.pourcentage())
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.code.cmp(&b.code))
        }),
        Ordre::Restantes => classeurs.sort_by(|a, b| {
            let manque = |c: &Classeur| c.total.saturating_sub(c.possedees);
            manque(b).cmp(&manque(a)).then_with(|| a.code.cmp(&b.code))
        }),
    }
}

/// Les classeurs que le terme cherché laisse passer.
///
/// Le terme est comparé au **code** et au **nom affiché**, sans casse. Un
/// terme vide laisse tout passer.
#[must_use]
pub fn filtrer<'a>(classeurs: &'a [Classeur], terme: &str, francais: bool) -> Vec<&'a Classeur> {
    let terme = terme.trim().to_lowercase();
    classeurs
        .iter()
        .filter(|c| {
            terme.is_empty()
                || c.code.to_lowercase().contains(&terme)
                || c.nom_affiche(francais).to_lowercase().contains(&terme)
        })
        .collect()
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

    /// Une installation vide, prête à recevoir ce que chaque test veut.
    fn installation() -> (tempfile::TempDir, Paths) {
        let dossier = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(dossier.path());
        std::fs::create_dir_all(paths.classeurs()).unwrap();
        std::fs::create_dir_all(paths.img_boosters()).unwrap();
        std::fs::create_dir_all(paths.img_small()).unwrap();
        (dossier, paths)
    }

    /// Un classeur avec `cartes` lignes, dont `possedees` possédées.
    fn classeur(paths: &Paths, code: &str, cartes: usize, possedees: usize) -> Connection {
        std::fs::create_dir_all(paths.dossier_classeur(code)).unwrap();
        let conn = ygo_db::connexion::ouvrir(paths.classeur_db(code)).unwrap();
        ygo_db::schema::creer_tout(&conn, ygo_db::schema::DDL_CLASSEUR).unwrap();
        for i in 0..cartes {
            conn.execute(
                "INSERT INTO cards (name, set_code, rarity, set_name, possessed, card_image_id) \
                 VALUES (?1, ?2, 'Ultra Rare', 'Le Set', ?3, ?4)",
                rusqlite::params![
                    format!("Carte {i}"),
                    format!("{code}-{i:03}"),
                    i64::from(i < possedees),
                    if i == 0 { Some(4_058_065_i64) } else { None },
                ],
            )
            .unwrap();
        }
        conn
    }

    // ── La grille ───────────────────────────────────────────────────────────

    #[test]
    fn la_grille_vient_de_la_table_meta_quand_elle_y_est() {
        // Les 26 classeurs réels sont tous en 3×3, soit la valeur par défaut :
        // sur eux, lire `meta` ou l'ignorer donne le même résultat. Ce test
        // emploie exprès une grille qui n'est pas celle par défaut.
        let (_d, paths) = installation();
        let conn = classeur(&paths, "RA05", 3, 1);
        conn.execute(
            "INSERT INTO meta (key, value) VALUES ('colonnes', '4'), ('lignes', '5')",
            [],
        )
        .unwrap();
        drop(conn);

        let liste = lister(&paths, (3, 3));
        assert_eq!((liste[0].colonnes, liste[0].lignes), (4, 5));
    }

    #[test]
    fn sans_table_meta_peuplee_la_grille_est_celle_par_defaut() {
        let (_d, paths) = installation();
        drop(classeur(&paths, "RA05", 3, 1));
        let liste = lister(&paths, (7, 2));
        assert_eq!((liste[0].colonnes, liste[0].lignes), (7, 2));
    }

    #[test]
    fn une_grille_illisible_retombe_sur_le_defaut_sans_echouer() {
        let (_d, paths) = installation();
        let conn = classeur(&paths, "RA05", 1, 0);
        conn.execute(
            "INSERT INTO meta (key, value) VALUES ('colonnes', 'quatre'), ('lignes', '')",
            [],
        )
        .unwrap();
        drop(conn);
        let liste = lister(&paths, (3, 3));
        assert_eq!((liste[0].colonnes, liste[0].lignes), (3, 3));
    }

    // ── Les quatre pistes de couverture ─────────────────────────────────────
    //
    // Aucune des trois dernières n'est exercée par l'oracle : les 26 classeurs
    // réels ont tous une cover de booster.

    #[test]
    fn la_cover_de_booster_passe_avant_la_carte() {
        let (_d, paths) = installation();
        drop(classeur(&paths, "RA05", 1, 0));
        std::fs::write(paths.cover_ext("RA05", "png"), b"x").unwrap();
        std::fs::write(paths.img_small().join("4058065.jpg"), b"x").unwrap();

        let liste = lister(&paths, (3, 3));
        assert_eq!(liste[0].origine_couverture, OrigineCouverture::Booster);
        assert_eq!(liste[0].couverture, Some(paths.cover_ext("RA05", "png")));
    }

    #[test]
    fn a_defaut_de_booster_la_carte_du_classeur_sert_de_couverture() {
        let (_d, paths) = installation();
        drop(classeur(&paths, "RA05", 1, 0));
        std::fs::write(paths.img_small().join("4058065.jpg"), b"x").unwrap();

        let liste = lister(&paths, (3, 3));
        assert_eq!(liste[0].origine_couverture, OrigineCouverture::Carte);
        assert_eq!(liste[0].image_id.as_deref(), Some("4058065"));
    }

    #[test]
    fn a_defaut_de_carte_l_ancien_dossier_par_classeur_sert_encore() {
        // Les classeurs YGOJSON historiques rangeaient leurs images ainsi.
        let (_d, paths) = installation();
        drop(classeur(&paths, "RA05", 1, 0));
        let ancien = paths.img().join("RA05");
        std::fs::create_dir_all(&ancien).unwrap();
        // Le `.png` est placé AVANT les `.jpg` dans l'ordre alphabétique :
        // c'est la seule disposition où le filtre d'extension se voit.
        for nom in ["aaa.png", "mmm.jpg", "zzz.jpg"] {
            std::fs::write(ancien.join(nom), b"x").unwrap();
        }

        let liste = lister(&paths, (3, 3));
        assert_eq!(liste[0].origine_couverture, OrigineCouverture::Dossier);
        assert_eq!(
            liste[0].couverture,
            Some(ancien.join("mmm.jpg")),
            "le premier .jpg par ordre alphabétique — un .png ne compte pas"
        );
    }

    #[test]
    fn sans_aucune_image_la_couverture_est_absente_et_le_dit() {
        let (_d, paths) = installation();
        drop(classeur(&paths, "RA05", 1, 0));
        let liste = lister(&paths, (3, 3));
        assert_eq!(liste[0].origine_couverture, OrigineCouverture::Aucune);
        assert_eq!(liste[0].couverture, None);
    }

    #[test]
    fn les_cinq_extensions_de_cover_sont_essayees_dans_l_ordre() {
        let (_d, paths) = installation();
        drop(classeur(&paths, "RA05", 1, 0));
        // Ni .png ni .jpg : c'est .webp qui doit être trouvé.
        std::fs::write(paths.cover_ext("RA05", "webp"), b"x").unwrap();
        assert_eq!(
            lister(&paths, (3, 3))[0].couverture,
            Some(paths.cover_ext("RA05", "webp"))
        );

        // .jpg arrive : il passe devant .webp.
        std::fs::write(paths.cover_ext("RA05", "jpg"), b"x").unwrap();
        assert_eq!(
            lister(&paths, (3, 3))[0].couverture,
            Some(paths.cover_ext("RA05", "jpg"))
        );
    }

    // ── La liste ────────────────────────────────────────────────────────────

    #[test]
    fn un_dossier_sans_base_reste_visible_a_zero_carte() {
        // C'est le résidu d'une suppression partielle. Le cacher priverait
        // l'utilisateur du seul endroit d'où il peut le supprimer.
        let (_d, paths) = installation();
        drop(classeur(&paths, "RA05", 3, 2));
        std::fs::create_dir_all(paths.dossier_classeur("VIDE")).unwrap();

        let liste = lister(&paths, (3, 3));
        assert_eq!(liste.len(), 2);
        let vide = liste.iter().find(|c| c.code == "VIDE").unwrap();
        assert_eq!(vide.total, 0);
        assert_eq!(vide.possedees, 0);
        assert_eq!(vide.nom, "VIDE", "faute de set_name, le code fait office");
        assert_eq!(vide.pourcentage(), 0.0, "et pas une division par zéro");
    }

    #[test]
    fn les_classeurs_sortent_dans_l_ordre_alphabetique() {
        let (_d, paths) = installation();
        for code in ["VASM", "EGO1", "RA05"] {
            drop(classeur(&paths, code, 1, 0));
        }
        let codes: Vec<String> = lister(&paths, (3, 3)).into_iter().map(|c| c.code).collect();
        assert_eq!(codes, ["EGO1", "RA05", "VASM"]);
    }

    #[test]
    fn les_totaux_additionnent_les_deux_colonnes() {
        let (_d, paths) = installation();
        drop(classeur(&paths, "AAA1", 10, 4));
        drop(classeur(&paths, "BBB1", 5, 5));
        assert_eq!(totaux(&lister(&paths, (3, 3))), (15, 9));
        assert_eq!(totaux(&[]), (0, 0));
    }

    #[test]
    fn un_fichier_dans_le_dossier_des_classeurs_n_est_pas_un_classeur() {
        let (_d, paths) = installation();
        drop(classeur(&paths, "RA05", 1, 0));
        std::fs::write(paths.classeurs().join("notes.txt"), b"x").unwrap();
        assert_eq!(lister(&paths, (3, 3)).len(), 1);
    }

    // ── Le nom du set ───────────────────────────────────────────────────────

    #[test]
    fn le_classeur_fait_autorite_sur_son_nom() {
        // Avec un `set_name`, `cardinfo.db` n'est même pas consultée — c'est
        // pourquoi elle peut être `None` ici sans que rien ne manque.
        assert_eq!(
            titre_set(None, "RA05", Some("Le Set"), false).as_deref(),
            Some("Le Set")
        );
    }

    #[test]
    fn lister_ne_consulte_cardinfo_que_pour_un_classeur_sans_set_name() {
        // Le classeur porte son nom : `lister` doit s'en contenter. La preuve
        // par l'absurde — il n'y a AUCUNE `cardinfo.db` sur ce disque, et le
        // nom sort quand même.
        let (_d, paths) = installation();
        drop(classeur(&paths, "RA05", 2, 1));
        assert!(!paths.cardinfo_db().exists());
        assert_eq!(lister(&paths, (3, 3))[0].nom, "Le Set");
    }

    #[test]
    fn sans_set_name_le_nom_vient_de_cardinfo() {
        let (_d, paths) = installation();
        let conn = classeur(&paths, "RA05", 1, 0);
        conn.execute("UPDATE cards SET set_name = NULL", [])
            .unwrap();
        drop(conn);

        std::fs::create_dir_all(paths.bdd()).unwrap();
        let cardinfo = ygo_db::connexion::ouvrir(paths.cardinfo_db()).unwrap();
        cardinfo
            .execute_batch(
                "CREATE TABLE sets (uuid TEXT, name_en TEXT, name_fr TEXT);\
                 CREATE TABLE set_locales (set_uuid TEXT, prefix TEXT);\
                 INSERT INTO sets VALUES ('u1', 'Rarity Collection 5', 'Collection Rareté 5');\
                 INSERT INTO set_locales VALUES ('u1', 'RA05-EN');",
            )
            .unwrap();
        drop(cardinfo);

        let liste = lister(&paths, (3, 3));
        assert_eq!(liste[0].nom, "Rarity Collection 5");
        assert_eq!(
            liste[0].nom_fr, "Collection Rareté 5",
            "c'est le SEUL chemin par lequel le nom français est atteignable"
        );
    }

    #[test]
    fn un_prefixe_ocg_est_trouve_par_egalite_et_non_par_like() {
        // `LOCR-JP` porte déjà son suffixe de langue : sa locale est exactement
        // `LOCR-JP`, et un `LIKE 'LOCR-JP-%'` ne la trouverait pas.
        let (_d, paths) = installation();
        let conn = classeur(&paths, "LOCR-JP", 1, 0);
        conn.execute("UPDATE cards SET set_name = NULL", [])
            .unwrap();
        drop(conn);

        std::fs::create_dir_all(paths.bdd()).unwrap();
        let cardinfo = ygo_db::connexion::ouvrir(paths.cardinfo_db()).unwrap();
        cardinfo
            .execute_batch(
                "CREATE TABLE sets (uuid TEXT, name_en TEXT, name_fr TEXT);\
                 CREATE TABLE set_locales (set_uuid TEXT, prefix TEXT);\
                 INSERT INTO sets VALUES ('u1', 'Limit Over Collection', NULL);\
                 INSERT INTO set_locales VALUES ('u1', 'LOCR-JP');",
            )
            .unwrap();
        drop(cardinfo);

        assert_eq!(lister(&paths, (3, 3))[0].nom, "Limit Over Collection");
    }

    #[test]
    fn le_nom_francais_est_ignore_des_que_le_classeur_porte_un_set_name() {
        // ÉCART REPRODUIT, pas corrigé. `get_set_title` rend le `set_name` du
        // classeur sans jamais consulter `use_fr` : sur tout classeur migré —
        // c'est-à-dire tous les 26 réels — l'interface française affiche le nom
        // ANGLAIS du set. Le nom français n'est atteignable que par le repli
        // sur `cardinfo.db`, donc seulement pour un classeur d'avant la
        // migration. Bug latent consigné au §4.3, à corriger après bascule.
        assert_eq!(
            titre_set(None, "RA05", Some("Le Set"), true),
            titre_set(None, "RA05", Some("Le Set"), false),
            "la langue ne change rien — c'est le comportement du Python"
        );
    }

    #[test]
    fn sans_set_name_ni_cardinfo_il_n_y_a_pas_de_nom() {
        assert_eq!(titre_set(None, "INCONNU", None, false), None);
        assert_eq!(titre_set(None, "INCONNU", Some(""), false), None);
    }

    #[test]
    fn une_base_illisible_ne_fait_pas_echouer_l_accueil() {
        // Un classeur corrompu ne doit pas empêcher l'écran de s'afficher :
        // il apparaît à zéro, et les autres restent lisibles.
        let (_d, paths) = installation();
        drop(classeur(&paths, "BON1", 4, 2));
        std::fs::create_dir_all(paths.dossier_classeur("CASSE")).unwrap();
        std::fs::write(
            paths.classeur_db("CASSE"),
            b"ceci n'est pas une base SQLite",
        )
        .unwrap();

        let liste = lister(&paths, (3, 3));
        assert_eq!(liste.len(), 2);
        assert_eq!(liste[0].code, "BON1");
        assert_eq!(liste[0].total, 4);
        assert_eq!(liste[1].code, "CASSE");
        assert_eq!(liste[1].total, 0);
    }

    #[test]
    fn le_pourcentage_est_derive_et_non_stocke() {
        let (_d, paths) = installation();
        drop(classeur(&paths, "RA05", 8, 3));
        let c = &lister(&paths, (3, 3))[0];
        assert!((c.pourcentage() - 37.5).abs() < 1e-9);
    }

    #[test]
    fn le_nom_affiche_suit_la_langue_demandee() {
        let c = Classeur {
            code: "RA05".to_owned(),
            nom: "Rarity Collection 5".to_owned(),
            nom_fr: "Collection Rareté 5".to_owned(),
            total: 1,
            possedees: 0,
            colonnes: 3,
            lignes: 3,
            image_id: None,
            couverture: None,
            origine_couverture: OrigineCouverture::Aucune,
        };
        assert_eq!(c.nom_affiche(false), "Rarity Collection 5");
        assert_eq!(c.nom_affiche(true), "Collection Rareté 5");
    }

    fn classeur_test(code: &str, total: usize, possedees: usize, nom: &str) -> Classeur {
        Classeur {
            code: code.to_owned(),
            nom: nom.to_owned(),
            nom_fr: format!("{nom} (fr)"),
            total,
            possedees,
            colonnes: 3,
            lignes: 3,
            image_id: None,
            couverture: None,
            origine_couverture: OrigineCouverture::Aucune,
        }
    }

    fn trois() -> Vec<Classeur> {
        vec![
            classeur_test("RA02", 100, 20, "Rarity Collection II"),
            classeur_test("EGO1", 40, 40, "Egyptian God Deck"),
            classeur_test("LDK2", 130, 65, "Legendary Decks II"),
        ]
    }

    #[test]
    fn l_ordre_par_defaut_est_celui_du_python() {
        let mut c = trois();
        trier(&mut c, Ordre::Code);
        assert_eq!(
            c.iter().map(|c| c.code.as_str()).collect::<Vec<_>>(),
            ["EGO1", "LDK2", "RA02"]
        );
        assert_eq!(Ordre::default(), Ordre::Code);
    }

    #[test]
    fn l_ordre_par_completion_met_les_plus_avances_devant() {
        let mut c = trois();
        trier(&mut c, Ordre::Completion);
        assert_eq!(
            c.iter().map(|c| c.code.as_str()).collect::<Vec<_>>(),
            ["EGO1", "LDK2", "RA02"],
            "100 %, 50 %, 20 %"
        );
    }

    /// L'ordre par manque n'est **pas** l'inverse de l'ordre par complétion,
    /// et c'est toute la raison de l'avoir en plus.
    ///
    /// # Le jeu d'essai a dû être refait
    ///
    /// Écrit d'abord sur les trois classeurs de [`trois`], ce test passait
    /// pour une mauvaise raison : sur ces trois-là, les deux ordres rendent
    /// **la même** liste, et l'assertion « les deux diffèrent » échouait.
    /// Il faut un gros classeur à moitié fait — beaucoup de manquantes malgré
    /// une complétion moyenne — face à un petit presque terminé.
    #[test]
    fn l_ordre_par_manque_n_est_pas_l_inverse_de_la_completion() {
        let jeu = || {
            vec![
                classeur_test("GROS", 1000, 500, "Gros"), //  50 %, 500 manquantes
                classeur_test("PETIT", 10, 9, "Petit"),   //  90 %,   1 manquante
                classeur_test("MOYEN", 100, 20, "Moyen"), //  20 %,  80 manquantes
            ]
        };
        let codes = |c: &[Classeur]| c.iter().map(|c| c.code.clone()).collect::<Vec<_>>();

        let mut par_completion = jeu();
        trier(&mut par_completion, Ordre::Completion);
        assert_eq!(codes(&par_completion), ["PETIT", "GROS", "MOYEN"]);

        let mut par_manque = jeu();
        trier(&mut par_manque, Ordre::Restantes);
        assert_eq!(codes(&par_manque), ["GROS", "MOYEN", "PETIT"]);

        let mut inverse = par_completion;
        inverse.reverse();
        assert_ne!(
            codes(&inverse),
            codes(&par_manque),
            "le manque n'est pas la complétion à l'envers"
        );
    }

    /// À complétion égale, le code départage : deux lancements donnent le même
    /// écran.
    #[test]
    fn le_code_departage_les_ex_aequo() {
        let mut c = vec![
            classeur_test("ZZZ", 10, 10, "Z"),
            classeur_test("AAA", 20, 20, "A"),
        ];
        trier(&mut c, Ordre::Completion);
        assert_eq!(
            c.iter().map(|c| c.code.as_str()).collect::<Vec<_>>(),
            ["AAA", "ZZZ"]
        );
    }

    #[test]
    fn un_classeur_vide_ne_fait_pas_exploser_le_tri() {
        let mut c = vec![
            classeur_test("VIDE", 0, 0, "V"),
            classeur_test("A", 4, 2, "A"),
        ];
        trier(&mut c, Ordre::Completion);
        assert_eq!(c[0].code, "A", "50 % passe devant 0 %");
        trier(&mut c, Ordre::Restantes);
        assert_eq!(c[0].code, "A", "2 manquantes passent devant 0");
    }

    #[test]
    fn le_filtre_porte_sur_le_code_et_sur_le_nom_affiche() {
        let c = trois();
        assert_eq!(filtrer(&c, "", false).len(), 3, "terme vide : tout passe");
        assert_eq!(filtrer(&c, "  ", false).len(), 3, "espaces aussi");
        assert_eq!(filtrer(&c, "ra0", false).len(), 1);
        assert_eq!(filtrer(&c, "RA0", false).len(), 1, "sans casse");
        assert_eq!(
            filtrer(&c, "legendary", false).len(),
            1,
            "sur le nom anglais"
        );
        assert_eq!(filtrer(&c, "zzz", false).len(), 0);
    }

    /// Le filtre suit la langue affichée : chercher le nom français ne doit
    /// rien donner quand l'écran est en anglais, et l'inverse.
    #[test]
    fn le_filtre_suit_la_langue_affichee() {
        let c = trois();
        assert_eq!(filtrer(&c, "(fr)", true).len(), 3);
        assert_eq!(filtrer(&c, "(fr)", false).len(), 0);
    }
}
