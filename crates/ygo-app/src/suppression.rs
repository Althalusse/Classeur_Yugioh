// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Retirer un classeur, sans le perdre.
//!
//! Aucun équivalent Python : la V1 ne sait pas supprimer un classeur depuis
//! l'application. C'est donc une fonction neuve, et la première de tout le
//! portage qui **détruit** quelque chose de l'utilisateur.
//!
//! # Pourquoi une corbeille et non un effacement
//!
//! Les quantités possédées ne vivent nulle part ailleurs que dans la base du
//! classeur. Supprimer `RA05`, c'est 176 exemplaires qui n'existent plus, sans
//! export, sans historique, sans annulation. Un mauvais clic sur une tuile
//! voisine coûterait des heures de saisie.
//!
//! Le dossier est donc **déplacé** vers `bdd/corbeille/<CODE>_<horodatage>/`.
//! Un `rename` sur le même volume est atomique et instantané, quelle que soit
//! la taille du classeur — et il se défait en remettant le dossier en place.
//!
//! # Ce qui n'est jamais touché
//!
//! `img/small` est **partagé** entre tous les classeurs : la même carte sert à
//! plusieurs sets. Y toucher casserait les autres. La couverture
//! `img/boosters/<CODE>.png` n'est partagée avec personne, mais elle reste
//! elle aussi : elle pèse peu, et un classeur recréé la retrouve sans nouveau
//! téléchargement.

use std::path::{Path, PathBuf};

use crate::error::{AppError, Result};
use ygo_core::paths::Paths;

/// Le dossier de corbeille d'une installation.
#[must_use]
pub fn corbeille(paths: &Paths) -> PathBuf {
    paths.bdd().join("corbeille")
}

/// Le nom que prend un classeur dans la corbeille.
///
/// `<CODE>_<AAAA-MM-JJ_hhmmss>` — l'horodatage évite qu'une seconde
/// suppression du même code écrase la première, et dit quand elle a eu lieu.
#[must_use]
pub fn nom_dans_corbeille(code: &str, horodatage: &str) -> String {
    format!("{}_{horodatage}", code.trim().to_uppercase())
}

/// L'horodatage du moment présent, au format employé par la corbeille.
#[must_use]
pub fn horodatage_maintenant() -> String {
    chrono::Local::now().format("%Y-%m-%d_%H%M%S").to_string()
}

/// Ce qu'une suppression emporterait — de quoi écrire une confirmation
/// honnête.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Consequence {
    /// Le code du classeur.
    pub code: String,
    /// Nombre de lignes.
    pub lignes: usize,
    /// Nombre de lignes portant une quantité.
    pub lignes_possedees: usize,
    /// Somme des quantités — c'est le chiffre qui compte pour l'utilisateur.
    pub exemplaires: i64,
}

impl Consequence {
    /// Y a-t-il quelque chose à perdre ?
    #[must_use]
    pub fn irreversible(&self) -> bool {
        self.exemplaires > 0
    }
}

/// Ce que la suppression de ce classeur emporterait.
///
/// Lit la base **en lecture seule**. Un classeur illisible rend des compteurs
/// à zéro plutôt qu'une erreur : on doit pouvoir supprimer un classeur
/// corrompu, c'est même souvent la raison de le supprimer.
#[must_use]
pub fn consequence(paths: &Paths, code: &str) -> Consequence {
    let code = code.trim().to_uppercase();
    let vide = Consequence {
        code: code.clone(),
        lignes: 0,
        lignes_possedees: 0,
        exemplaires: 0,
    };
    let Ok(conn) = ygo_db::connexion::ouvrir_lecture_seule(paths.classeur_db(&code)) else {
        return vide;
    };
    conn.query_row(
        "SELECT COUNT(*), \
                COALESCE(SUM(CASE WHEN COALESCE(quantite, 0) > 0 THEN 1 ELSE 0 END), 0), \
                COALESCE(SUM(COALESCE(quantite, 0)), 0) \
         FROM cards",
        [],
        |l| {
            Ok(Consequence {
                code: code.clone(),
                lignes: l.get::<_, i64>(0)?.max(0) as usize,
                lignes_possedees: l.get::<_, i64>(1)?.max(0) as usize,
                exemplaires: l.get(2)?,
            })
        },
    )
    .unwrap_or(vide)
}

/// Ce qu'une suppression fait du dossier.
///
/// Le Python n'avait qu'un geste : à la corbeille, puis un second passage par
/// l'écran de la corbeille pour effacer. Deux gestes pour un classeur dont on
/// sait déjà, en cliquant, qu'on n'en veut plus — et deux gestes qui se
/// terminent au même endroit. Sur demande de l'utilisateur (2026-09-20), le
/// choix se fait **au moment où il se pose**.
///
/// Le défaut reste la corbeille : c'est le geste qu'on peut défaire, et il ne
/// coûte rien. [`Definitif`](Self::Definitif) est offert à côté, jamais à sa
/// place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Déplacer dans `bdd/corbeille/` — réversible.
    Corbeille,
    /// Effacer le dossier du disque — sans retour.
    Definitif,
}

impl Mode {
    /// Le libellé du bouton qui déclenche ce mode.
    #[must_use]
    pub fn libelle(self) -> &'static str {
        match self {
            Self::Corbeille => "Mettre à la corbeille",
            Self::Definitif => "Supprimer définitivement",
        }
    }

    /// Ce qu'il advient du classeur, en une phrase.
    #[must_use]
    pub fn consequence(self) -> &'static str {
        match self {
            Self::Corbeille => {
                "Le classeur est déplacé dans bdd/corbeille/ — rien n'est effacé, \
                 vous pouvez le remettre en place."
            }
            Self::Definitif => {
                "Le dossier du classeur est effacé du disque. Il n'y a pas de retour, \
                 et la corbeille ne le contiendra pas."
            }
        }
    }

    /// Ce mode détruit-il des données sans recours ?
    #[must_use]
    pub fn est_sans_retour(self) -> bool {
        matches!(self, Self::Definitif)
    }
}

/// Efface le dossier d'un classeur **sans passer par la corbeille**.
///
/// # Ce que cette fonction ne fait pas
///
/// Elle ne touche ni aux images de cartes, ni à la couverture, ni aux
/// anomalies connues — exactement comme [`vers_corbeille`]. Seul le dossier du
/// classeur disparaît. Une recréation le rebâtit depuis les sources ; ce qui
/// est perdu sans retour, ce sont les **quantités saisies**, et c'est à
/// l'appelant de l'avoir dit avant d'arriver ici.
///
/// # Errors
///
/// Si le classeur n'existe pas, ou si l'effacement échoue.
pub fn effacer(paths: &Paths, code: &str) -> Result<PathBuf> {
    let code = code.trim().to_uppercase();
    let cible = paths.dossier_classeur(&code);
    if !cible.is_dir() {
        return Err(AppError::Creation(format!(
            "{code} : aucun dossier de classeur à supprimer"
        )));
    }
    std::fs::remove_dir_all(&cible)
        .map_err(|e| AppError::Creation(format!("effacement de {code} : {e}")))?;
    Ok(cible)
}

/// Supprime un classeur selon le mode demandé.
///
/// Le point d'entrée unique : l'interface dit *quoi*, ce module fait *comment*.
/// Rend le chemin concerné — la destination pour une mise à la corbeille, le
/// dossier effacé pour une suppression définitive.
///
/// # Errors
///
/// Celles de [`vers_corbeille`] ou d'[`effacer`], selon le mode.
pub fn supprimer(paths: &Paths, code: &str, mode: Mode, horodatage: &str) -> Result<PathBuf> {
    match mode {
        Mode::Corbeille => vers_corbeille(paths, code, horodatage),
        Mode::Definitif => effacer(paths, code),
    }
}

/// Déplace un classeur vers la corbeille.
///
/// Rend le chemin où il a atterri.
///
/// # Errors
///
/// Si le classeur n'existe pas, ou si le déplacement échoue — base ouverte par
/// une autre instance, disque plein, permissions.
pub fn vers_corbeille(paths: &Paths, code: &str, horodatage: &str) -> Result<PathBuf> {
    let code = code.trim().to_uppercase();
    let source = paths.dossier_classeur(&code);
    if !source.is_dir() {
        return Err(AppError::Creation(format!(
            "{code} : aucun dossier de classeur à supprimer"
        )));
    }
    let destination = corbeille(paths).join(nom_dans_corbeille(&code, horodatage));
    if destination.exists() {
        return Err(AppError::Creation(format!(
            "{} existe déjà",
            destination.display()
        )));
    }
    std::fs::create_dir_all(corbeille(paths))
        .map_err(|e| AppError::Creation(format!("corbeille : {e}")))?;
    deplacer(&source, &destination)?;
    Ok(destination)
}

/// Déplace un dossier, avec repli sur copie puis effacement.
///
/// `rename` est atomique et instantané sur un même volume, et c'est le cas
/// courant — la corbeille est dans `bdd/`, à côté des classeurs. Il échoue si
/// les deux se trouvent sur des volumes différents ; le repli existe pour ce
/// cas-là, qu'un dossier `bdd` monté ailleurs suffit à créer.
fn deplacer(source: &Path, destination: &Path) -> Result<()> {
    if std::fs::rename(source, destination).is_ok() {
        return Ok(());
    }
    copier_recursif(source, destination)
        .map_err(|e| AppError::Creation(format!("copie vers la corbeille : {e}")))?;
    std::fs::remove_dir_all(source)
        .map_err(|e| AppError::Creation(format!("effacement de l'original : {e}")))
}

fn copier_recursif(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(destination)?;
    for entree in std::fs::read_dir(source)? {
        let entree = entree?;
        let cible = destination.join(entree.file_name());
        if entree.file_type()?.is_dir() {
            copier_recursif(&entree.path(), &cible)?;
        } else {
            std::fs::copy(entree.path(), &cible)?;
        }
    }
    Ok(())
}

/// Ce que la corbeille contient.
///
/// Rendu trié, le plus récent d'abord — c'est ce qu'on cherche quand on vient
/// d'effacer par erreur.
#[must_use]
pub fn contenu_corbeille(paths: &Paths) -> Vec<String> {
    let Ok(entrees) = std::fs::read_dir(corbeille(paths)) else {
        return Vec::new();
    };
    let mut noms: Vec<String> = entrees
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    noms.sort_by(|a, b| b.cmp(a));
    noms
}

/// Une entrée de la corbeille, telle que l'écran la montre.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rebut {
    /// Le nom du dossier — `RA05_2026-08-28_120000`.
    pub dossier: String,
    /// Le code du classeur, extrait du nom.
    pub code: String,
    /// La date de suppression, telle qu'elle est lisible.
    pub date: String,
    /// Nombre de lignes.
    pub lignes: usize,
    /// Somme des quantités.
    pub exemplaires: i64,
    /// Un classeur du même code existe-t-il déjà ? La restauration serait
    /// alors refusée.
    pub code_repris: bool,
}

/// Sépare le nom de dossier en code et horodatage.
///
/// Le code peut contenir des tirets — `LOCH-JP` — mais jamais de
/// souligné : la coupure se fait sur le **premier** souligné, qui est celui
/// que [`nom_dans_corbeille`] a posé.
#[must_use]
pub fn separer(dossier: &str) -> (String, String) {
    match dossier.split_once('_') {
        Some((code, horodatage)) => (code.to_owned(), horodatage.replace('_', " à ")),
        None => (dossier.to_owned(), String::new()),
    }
}

/// Le contenu de la corbeille, détaillé.
#[must_use]
pub fn rebuts(paths: &Paths) -> Vec<Rebut> {
    contenu_corbeille(paths)
        .into_iter()
        .map(|dossier| {
            let (code, date) = separer(&dossier);
            let base = corbeille(paths).join(&dossier).join(format!("{code}.db"));
            let (lignes, exemplaires) = compter(&base);
            Rebut {
                code_repris: paths.dossier_classeur(&code).is_dir(),
                dossier,
                code,
                date,
                lignes,
                exemplaires,
            }
        })
        .collect()
}

/// Lit les compteurs d'une base de classeur, où qu'elle soit.
fn compter(base: &Path) -> (usize, i64) {
    let Ok(conn) = ygo_db::connexion::ouvrir_lecture_seule(base) else {
        return (0, 0);
    };
    conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(COALESCE(quantite, 0)), 0) FROM cards",
        [],
        |l| Ok((l.get::<_, i64>(0)?.max(0) as usize, l.get(1)?)),
    )
    .unwrap_or((0, 0))
}

/// Remet un classeur de la corbeille à sa place.
///
/// # Errors
///
/// Si l'entrée n'existe pas, ou si un classeur du même code est déjà en place
/// — on ne remplace jamais un classeur vivant par un mort : ce serait perdre
/// les quantités saisies depuis la suppression.
pub fn restaurer(paths: &Paths, dossier: &str) -> Result<PathBuf> {
    let source = corbeille(paths).join(dossier);
    if !source.is_dir() {
        return Err(AppError::Creation(format!(
            "{dossier} : introuvable dans la corbeille"
        )));
    }
    let (code, _) = separer(dossier);
    let destination = paths.dossier_classeur(&code);
    if destination.exists() {
        return Err(AppError::Creation(format!(
            "{code} existe déjà — renommez ou supprimez-le avant de restaurer"
        )));
    }
    std::fs::create_dir_all(paths.classeurs())
        .map_err(|e| AppError::Creation(format!("dossier des classeurs : {e}")))?;
    deplacer(&source, &destination)?;
    Ok(destination)
}

/// Efface définitivement une entrée de la corbeille.
///
/// # Errors
///
/// Si l'entrée n'existe pas, ou si l'effacement échoue.
pub fn vider(paths: &Paths, dossier: &str) -> Result<()> {
    let cible = corbeille(paths).join(dossier);
    if !cible.is_dir() {
        return Err(AppError::Creation(format!(
            "{dossier} : introuvable dans la corbeille"
        )));
    }
    std::fs::remove_dir_all(&cible)
        .map_err(|e| AppError::Creation(format!("effacement de {dossier} : {e}")))
}

/// Ce qu'un vidage complet emporterait.
///
/// Se lit **avant** de vider, pour que la confirmation dise ce qu'elle
/// détruit plutôt qu'un nombre de dossiers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ardoise {
    /// Les entrées, dans l'ordre où l'écran les montre.
    pub rebuts: Vec<Rebut>,
    /// Total des lignes de classeur.
    pub lignes: usize,
    /// Total des exemplaires possédés — c'est le seul chiffre irremplaçable.
    pub exemplaires: i64,
}

impl Ardoise {
    /// Combien d'entrées.
    #[must_use]
    pub fn entrees(&self) -> usize {
        self.rebuts.len()
    }

    /// Y a-t-il quelque chose à perdre, au-delà des lignes elles-mêmes ?
    #[must_use]
    pub fn irreversible(&self) -> bool {
        self.exemplaires > 0
    }

    /// Les codes concernés, dédoublonnés et triés — de quoi les nommer sans
    /// dérouler la liste entière.
    #[must_use]
    pub fn codes(&self) -> Vec<String> {
        let mut v: Vec<String> = self.rebuts.iter().map(|r| r.code.clone()).collect();
        v.sort_unstable();
        v.dedup();
        v
    }
}

/// Ce que la corbeille contient, totalisé.
#[must_use]
pub fn ardoise(paths: &Paths) -> Ardoise {
    let rebuts = rebuts(paths);
    Ardoise {
        lignes: rebuts.iter().map(|r| r.lignes).sum(),
        exemplaires: rebuts.iter().map(|r| r.exemplaires).sum(),
        rebuts,
    }
}

/// Ce qu'un vidage complet a fait, et ce qu'il n'a pas pu faire.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BilanVidage {
    /// Les entrées effacées.
    pub effacees: Vec<String>,
    /// Celles qui ont résisté, et pourquoi.
    pub echecs: Vec<(String, String)>,
}

impl BilanVidage {
    /// Tout est parti ?
    #[must_use]
    pub fn complet(&self) -> bool {
        self.echecs.is_empty()
    }
}

/// Efface définitivement **toute** la corbeille.
///
/// # Pourquoi ça n'échoue pas au premier obstacle
///
/// Un dossier verrouillé par l'explorateur Windows, ou une base encore
/// ouverte, fait échouer *son* effacement et rien d'autre. S'arrêter là
/// laisserait une corbeille à moitié vide sans dire laquelle des vingt
/// entrées a bloqué — et un second clic recommencerait tout depuis le début.
/// Chaque entrée est donc tentée, et le bilan **nomme** celles qui restent.
///
/// Le dossier `corbeille/` lui-même n'est pas retiré : il est recréé au
/// prochain passage à la corbeille, et son absence ferait échouer les
/// lectures qui l'ouvrent sans le créer.
pub fn vider_tout(paths: &Paths) -> BilanVidage {
    let mut bilan = BilanVidage::default();
    for dossier in contenu_corbeille(paths) {
        match vider(paths, &dossier) {
            Ok(()) => bilan.effacees.push(dossier),
            Err(e) => bilan.echecs.push((dossier, e.to_string())),
        }
    }
    bilan
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;
    use rusqlite::Connection;

    fn installation() -> (tempfile::TempDir, Paths) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(tmp.path());
        std::fs::create_dir_all(paths.classeurs()).unwrap();
        (tmp, paths)
    }

    fn classeur(paths: &Paths, code: &str, quantites: &[i64]) {
        std::fs::create_dir_all(paths.dossier_classeur(code)).unwrap();
        let conn = Connection::open(paths.classeur_db(code)).unwrap();
        conn.execute("CREATE TABLE cards (quantite INTEGER)", ())
            .unwrap();
        for q in quantites {
            conn.execute("INSERT INTO cards VALUES (?1)", [q]).unwrap();
        }
    }

    #[test]
    fn la_consequence_compte_les_lignes_et_les_exemplaires() {
        let (_tmp, paths) = installation();
        classeur(&paths, "RA05", &[0, 1, 3, 0, 2]);

        let c = consequence(&paths, "RA05");
        assert_eq!(c.lignes, 5);
        assert_eq!(c.lignes_possedees, 3, "trois lignes portent une quantité");
        assert_eq!(c.exemplaires, 6, "1 + 3 + 2 — c'est ce qui se perdrait");
        assert!(c.irreversible());
    }

    /// Un classeur sans une seule carte possédée se supprime sans regret : la
    /// confirmation peut le dire, au lieu d'alarmer pour rien.
    #[test]
    fn un_classeur_vide_n_est_pas_une_perte() {
        let (_tmp, paths) = installation();
        classeur(&paths, "NEUF", &[0, 0, 0]);
        let c = consequence(&paths, "NEUF");
        assert_eq!(c.lignes, 3);
        assert_eq!(c.exemplaires, 0);
        assert!(!c.irreversible());
    }

    /// On doit pouvoir supprimer un classeur illisible — c'est même souvent la
    /// raison de le supprimer. Les compteurs tombent à zéro, sans erreur.
    #[test]
    fn un_classeur_illisible_se_laisse_examiner_sans_lever() {
        let (_tmp, paths) = installation();
        std::fs::create_dir_all(paths.dossier_classeur("CASSE")).unwrap();
        std::fs::write(paths.classeur_db("CASSE"), b"pas une base").unwrap();

        let c = consequence(&paths, "CASSE");
        assert_eq!(c.code, "CASSE");
        assert_eq!(c.lignes, 0);
        assert!(!c.irreversible());
        // Et il part quand même à la corbeille.
        assert!(vers_corbeille(&paths, "CASSE", "2026-08-28_120000").is_ok());
    }

    #[test]
    fn le_classeur_part_a_la_corbeille_et_disparait_de_sa_place() {
        let (_tmp, paths) = installation();
        classeur(&paths, "RA05", &[1, 2]);
        assert!(paths.dossier_classeur("RA05").is_dir());

        let ou = vers_corbeille(&paths, "RA05", "2026-08-28_120000").unwrap();
        assert!(!paths.dossier_classeur("RA05").exists(), "plus à sa place");
        assert!(ou.is_dir(), "mais dans la corbeille");
        assert!(ou.join("RA05.db").is_file(), "avec sa base");
        assert_eq!(
            ou.file_name().unwrap().to_string_lossy(),
            "RA05_2026-08-28_120000"
        );
    }

    /// La seconde sortie, demandée le 2026-09-20 : effacer sans passer par la
    /// corbeille.
    #[test]
    fn la_suppression_definitive_n_ecrit_rien_dans_la_corbeille() {
        let (_tmp, paths) = installation();
        classeur(&paths, "RA05", &[1, 2]);

        let efface = effacer(&paths, "RA05").unwrap();
        assert!(!paths.dossier_classeur("RA05").exists(), "plus à sa place");
        assert!(!efface.exists(), "ni ailleurs");
        assert!(
            contenu_corbeille(&paths).is_empty(),
            "la corbeille ne le recueille pas — c'est tout l'objet du geste"
        );
    }

    /// Le mode est le seul écart entre les deux chemins : même entrée, même
    /// garde, deux destinations.
    #[test]
    fn le_mode_choisit_la_destination() {
        let (_tmp, paths) = installation();
        classeur(&paths, "RA05", &[1]);
        classeur(&paths, "RA02", &[1]);

        supprimer(&paths, "RA05", Mode::Corbeille, "2026-09-20_210000").unwrap();
        supprimer(&paths, "RA02", Mode::Definitif, "2026-09-20_210000").unwrap();

        assert_eq!(
            contenu_corbeille(&paths),
            vec!["RA05_2026-09-20_210000".to_owned()],
            "seul le premier y est"
        );
        assert!(!paths.dossier_classeur("RA05").exists());
        assert!(!paths.dossier_classeur("RA02").exists());
    }

    /// Un classeur absent se refuse des deux côtés, et de la même façon : une
    /// erreur qui le nomme, jamais un succès silencieux.
    #[test]
    fn effacer_un_classeur_absent_le_dit_en_le_nommant() {
        let (_tmp, paths) = installation();
        let erreur = effacer(&paths, "FANTOME").unwrap_err().to_string();
        assert!(erreur.contains("FANTOME"), "{erreur}");
        assert!(
            vers_corbeille(&paths, "FANTOME", "2026-09-20_210000").is_err(),
            "et la corbeille aussi"
        );
    }

    /// Le code est normalisé avant tout, comme partout ailleurs.
    #[test]
    fn l_effacement_normalise_le_code() {
        let (_tmp, paths) = installation();
        classeur(&paths, "RA05", &[1]);
        assert!(effacer(&paths, "  ra05  ").is_ok());
        assert!(!paths.dossier_classeur("RA05").exists());
    }

    /// Les deux modes se présentent différemment — c'est ce qui permet à
    /// l'interface de les offrir sans les confondre.
    #[test]
    fn les_deux_modes_se_disent_distinctement() {
        assert_ne!(Mode::Corbeille.libelle(), Mode::Definitif.libelle());
        assert_ne!(Mode::Corbeille.consequence(), Mode::Definitif.consequence());
        assert!(!Mode::Corbeille.est_sans_retour());
        assert!(Mode::Definitif.est_sans_retour());
        assert!(
            Mode::Definitif.consequence().contains("pas de retour"),
            "la phrase du mode destructeur doit le dire"
        );
    }

    /// Le dossier déplacé garde tout son contenu — c'est ce qui rend
    /// l'opération réversible à la main.
    #[test]
    fn rien_ne_se_perd_dans_le_deplacement() {
        let (_tmp, paths) = installation();
        classeur(&paths, "RA05", &[1]);
        std::fs::write(
            paths.dossier_classeur("RA05").join("note.txt"),
            b"garde-moi",
        )
        .unwrap();

        let ou = vers_corbeille(&paths, "RA05", "h").unwrap();
        assert_eq!(std::fs::read(ou.join("note.txt")).unwrap(), b"garde-moi");
    }

    /// Deux suppressions du même code ne s'écrasent pas : l'horodatage les
    /// sépare.
    #[test]
    fn deux_suppressions_du_meme_code_cohabitent() {
        let (_tmp, paths) = installation();
        classeur(&paths, "RA05", &[1]);
        vers_corbeille(&paths, "RA05", "2026-08-28_120000").unwrap();

        classeur(&paths, "RA05", &[2]);
        vers_corbeille(&paths, "RA05", "2026-08-28_130000").unwrap();

        assert_eq!(
            contenu_corbeille(&paths),
            ["RA05_2026-08-28_130000", "RA05_2026-08-28_120000"],
            "le plus récent d'abord"
        );
    }

    /// Un horodatage identique ne doit **pas** écraser : mieux vaut refuser.
    #[test]
    fn une_collision_exacte_est_refusee_plutot_qu_ecrasee() {
        let (_tmp, paths) = installation();
        classeur(&paths, "RA05", &[1]);
        vers_corbeille(&paths, "RA05", "meme").unwrap();
        classeur(&paths, "RA05", &[2]);
        assert!(vers_corbeille(&paths, "RA05", "meme").is_err());
        assert!(
            paths.dossier_classeur("RA05").is_dir(),
            "et le classeur reste en place"
        );
    }

    /// Supprimer un classeur absent échoue — et le message **nomme le code**.
    ///
    /// Sans la garde explicite, l'opération échouerait quand même : le
    /// déplacement d'un dossier inexistant ne peut pas réussir. Ce que la garde
    /// apporte est le message. Une mutation qui la supprimait ne faisait donc
    /// échouer aucun test tant que celui-ci se contentait de `is_err()` — la
    /// garde n'était pas testée pour ce qu'elle fait vraiment.
    #[test]
    fn supprimer_un_classeur_absent_le_dit_en_le_nommant() {
        let (_tmp, paths) = installation();
        let erreur = vers_corbeille(&paths, "jamais-vu", "h")
            .unwrap_err()
            .to_string();
        assert!(erreur.contains("JAMAIS-VU"), "message : {erreur}");
        assert!(erreur.contains("aucun dossier"), "message : {erreur}");
    }

    /// Les images ne sont jamais touchées : `img/small` est partagé entre tous
    /// les classeurs, et la couverture sert si le classeur est recréé.
    #[test]
    fn les_images_ne_sont_pas_touchees() {
        let (_tmp, paths) = installation();
        classeur(&paths, "RA05", &[1]);
        std::fs::create_dir_all(paths.img_small()).unwrap();
        std::fs::create_dir_all(paths.img_boosters()).unwrap();
        let carte = paths.img_small().join("12345.jpg");
        let cover = paths.img_boosters().join("RA05.png");
        std::fs::write(&carte, b"image").unwrap();
        std::fs::write(&cover, b"cover").unwrap();

        vers_corbeille(&paths, "RA05", "h").unwrap();
        assert!(carte.is_file(), "les cartes sont partagées");
        assert!(cover.is_file(), "la couverture resservira");
    }

    #[test]
    fn l_horodatage_a_le_format_attendu() {
        let h = horodatage_maintenant();
        assert_eq!(h.len(), 17, "AAAA-MM-JJ_hhmmss");
        assert_eq!(nom_dans_corbeille("ra05", "h"), "RA05_h");
    }

    #[test]
    fn une_corbeille_absente_est_vide_et_ne_gene_pas() {
        let (_tmp, paths) = installation();
        assert!(contenu_corbeille(&paths).is_empty());
    }

    // ── Corbeille : détail, restauration, effacement ────────────────────────

    /// Le code peut contenir des tirets — `LOCH-JP` — mais jamais de souligné.
    /// La coupure se fait donc sur le **premier** souligné, celui que le nom
    /// de corbeille a posé.
    #[test]
    fn le_code_se_separe_de_l_horodatage_meme_avec_un_tiret() {
        assert_eq!(
            separer("LOCH-JP_2026-08-28_120000"),
            ("LOCH-JP".to_owned(), "2026-08-28 à 120000".to_owned())
        );
        assert_eq!(
            separer("RA05_2026-08-28_120000"),
            ("RA05".to_owned(), "2026-08-28 à 120000".to_owned())
        );
        assert_eq!(
            separer("SANS-HORODATAGE"),
            ("SANS-HORODATAGE".to_owned(), String::new())
        );
    }

    #[test]
    fn le_detail_de_la_corbeille_compte_ce_qui_dort_dedans() {
        let (_tmp, paths) = installation();
        classeur(&paths, "RA05", &[1, 2, 0]);
        vers_corbeille(&paths, "RA05", "2026-08-28_120000").unwrap();

        let r = rebuts(&paths);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].code, "RA05");
        assert_eq!(r[0].lignes, 3);
        assert_eq!(r[0].exemplaires, 3);
        assert!(!r[0].code_repris, "aucun RA05 vivant");
    }

    /// Un classeur recréé sous le même code interdit la restauration : la
    /// remettre écraserait ce qui a été saisi depuis. L'écran le sait avant de
    /// proposer le bouton.
    #[test]
    fn un_code_repris_est_signale_et_la_restauration_refusee() {
        let (_tmp, paths) = installation();
        classeur(&paths, "RA05", &[1]);
        vers_corbeille(&paths, "RA05", "h").unwrap();
        classeur(&paths, "RA05", &[5, 5]);

        let r = rebuts(&paths);
        assert!(r[0].code_repris);
        assert!(restaurer(&paths, "RA05_h").is_err());
        // Et le classeur vivant n'a pas bougé.
        assert_eq!(consequence(&paths, "RA05").exemplaires, 10);
    }

    #[test]
    fn restaurer_remet_le_classeur_a_sa_place_et_vide_l_entree() {
        let (_tmp, paths) = installation();
        classeur(&paths, "RA05", &[1, 2]);
        vers_corbeille(&paths, "RA05", "h").unwrap();
        assert!(!paths.dossier_classeur("RA05").exists());

        let ou = restaurer(&paths, "RA05_h").unwrap();
        assert_eq!(ou, paths.dossier_classeur("RA05"));
        assert_eq!(
            consequence(&paths, "RA05").exemplaires,
            3,
            "quantités intactes"
        );
        assert!(
            contenu_corbeille(&paths).is_empty(),
            "l'entrée a quitté la corbeille"
        );
    }

    #[test]
    fn vider_efface_une_entree_et_laisse_les_autres() {
        let (_tmp, paths) = installation();
        classeur(&paths, "RA05", &[1]);
        vers_corbeille(&paths, "RA05", "a").unwrap();
        classeur(&paths, "LDK2", &[1]);
        vers_corbeille(&paths, "LDK2", "b").unwrap();

        vider(&paths, "RA05_a").unwrap();
        assert_eq!(contenu_corbeille(&paths), ["LDK2_b"]);
    }

    /// Une entrée absente est une erreur — et l'erreur **dit laquelle**.
    ///
    /// # Ce test s'est d'abord contenté de `is_err()`
    ///
    /// Sans les deux gardes, `restaurer` et `vider` échouent quand même :
    /// `rename` et `remove_dir_all` ne travaillent pas sur ce qui n'existe
    /// pas. Un `is_err()` passait donc avec ou sans elles, et deux mutations
    /// supprimant les gardes survivaient. Leur seul apport est le message :
    /// « introuvable dans la corbeille » au lieu d'un `os error 2` remonté
    /// depuis les entrailles. C'est cela qu'il faut affirmer.
    #[test]
    fn restaurer_ou_vider_une_entree_absente_le_dit_en_clair() {
        let (_tmp, paths) = installation();
        for erreur in [
            restaurer(&paths, "JAMAIS_VU").unwrap_err().to_string(),
            vider(&paths, "JAMAIS_VU").unwrap_err().to_string(),
        ] {
            assert!(
                erreur.contains("introuvable dans la corbeille"),
                "message opaque : {erreur}"
            );
            assert!(erreur.contains("JAMAIS_VU"), "il faut nommer : {erreur}");
        }
    }

    /// Ce qui n'est pas un dossier de classeur n'est pas une entrée de
    /// corbeille.
    ///
    /// `bdd/corbeille/` est un dossier ordinaire, que Windows garnit tout seul
    /// d'un `Thumbs.db` dès qu'on l'ouvre dans l'explorateur. Le compter
    /// afficherait « Corbeille (1) » sur une corbeille vide, et proposerait de
    /// « restaurer » un fichier.
    #[test]
    fn un_fichier_egare_dans_la_corbeille_n_est_pas_une_entree() {
        let (_tmp, paths) = installation();
        std::fs::create_dir_all(corbeille(&paths)).unwrap();
        std::fs::write(corbeille(&paths).join("Thumbs.db"), b"x").unwrap();
        assert!(contenu_corbeille(&paths).is_empty());
        assert!(rebuts(&paths).is_empty());

        classeur(&paths, "RA05", &[1]);
        vers_corbeille(&paths, "RA05", "2026-08-29_120000").unwrap();
        assert_eq!(
            contenu_corbeille(&paths).len(),
            1,
            "le vrai dossier, lui, compte"
        );
    }

    /// L'ardoise totalise ce qu'un vidage complet emporterait — et c'est
    /// **avant** de vider qu'elle se lit.
    #[test]
    fn l_ardoise_dit_ce_que_le_vidage_emporterait() {
        let (_tmp, paths) = installation();
        assert_eq!(ardoise(&paths), Ardoise::default(), "corbeille vide");
        assert!(!ardoise(&paths).irreversible());

        classeur(&paths, "RA05", &[1, 2, 3]);
        classeur(&paths, "SDLI", &[0, 0]);
        vers_corbeille(&paths, "RA05", "h1").unwrap();
        vers_corbeille(&paths, "SDLI", "h2").unwrap();

        let a = ardoise(&paths);
        assert_eq!(a.entrees(), 2);
        assert_eq!(a.lignes, 5, "3 + 2");
        assert_eq!(a.exemplaires, 6, "1 + 2 + 3, et rien pour SDLI");
        assert!(a.irreversible(), "des exemplaires sont en jeu");
        assert_eq!(a.codes(), vec!["RA05".to_owned(), "SDLI".to_owned()]);
    }

    /// Une corbeille qui ne contient que des classeurs vides ne fait pas
    /// peur pour rien : la confirmation le dira.
    #[test]
    fn une_corbeille_sans_exemplaire_n_est_pas_irreversible() {
        let (_tmp, paths) = installation();
        classeur(&paths, "SDLI", &[0, 0]);
        vers_corbeille(&paths, "SDLI", "h").unwrap();
        let a = ardoise(&paths);
        assert_eq!(a.lignes, 2);
        assert!(!a.irreversible());
    }

    /// Le vidage complet emporte tout, et le dossier de corbeille survit.
    #[test]
    fn le_vidage_complet_emporte_tout_et_garde_le_dossier() {
        let (_tmp, paths) = installation();
        for (code, h) in [("RA05", "h1"), ("SDLI", "h2"), ("EGO1", "h3")] {
            classeur(&paths, code, &[1]);
            vers_corbeille(&paths, code, h).unwrap();
        }
        assert_eq!(contenu_corbeille(&paths).len(), 3);

        let bilan = vider_tout(&paths);
        assert!(bilan.complet(), "{:?}", bilan.echecs);
        assert_eq!(bilan.effacees.len(), 3);
        assert!(contenu_corbeille(&paths).is_empty());
        assert!(
            corbeille(&paths).is_dir(),
            "le dossier reste : les lectures l'ouvrent sans le créer"
        );

        // Et rejouer sur une corbeille vide ne fait rien, sans se plaindre.
        let encore = vider_tout(&paths);
        assert!(encore.complet());
        assert!(encore.effacees.is_empty());
    }

    /// Ce qui n'est pas une entrée de corbeille n'est pas touché.
    ///
    /// Un fichier posé là à la main — un export, une note — n'est pas un
    /// classeur : [`contenu_corbeille`] ne retient que les dossiers, et le
    /// vidage ne voit que ce qu'elle lui donne.
    #[test]
    fn un_fichier_egare_dans_la_corbeille_survit() {
        let (_tmp, paths) = installation();
        classeur(&paths, "RA05", &[1]);
        vers_corbeille(&paths, "RA05", "h").unwrap();
        let egare = corbeille(&paths).join("note.txt");
        std::fs::write(&egare, b"pas un classeur").unwrap();

        let bilan = vider_tout(&paths);
        assert_eq!(bilan.effacees, vec!["RA05_h".to_owned()]);
        assert!(egare.is_file(), "le fichier est resté");
    }

    /// Un aller-retour complet ne perd rien : c'est toute la promesse de la
    /// corbeille.
    #[test]
    fn un_aller_retour_par_la_corbeille_ne_perd_rien() {
        let (_tmp, paths) = installation();
        classeur(&paths, "RA05", &[1, 2, 3]);
        std::fs::write(
            paths.dossier_classeur("RA05").join("note.txt"),
            b"garde-moi",
        )
        .unwrap();
        let avant = consequence(&paths, "RA05");

        vers_corbeille(&paths, "RA05", "h").unwrap();
        restaurer(&paths, "RA05_h").unwrap();

        assert_eq!(consequence(&paths, "RA05"), avant);
        assert_eq!(
            std::fs::read(paths.dossier_classeur("RA05").join("note.txt")).unwrap(),
            b"garde-moi"
        );
    }
}
