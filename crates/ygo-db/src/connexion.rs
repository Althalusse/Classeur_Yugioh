// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Ouverture des bases et sérialisation des accès.
//!
//! Portage de `centralisation_dossier.sqlite_ctx()` et de son verrou par base
//! (`_db_lock`).
//!
//! # Ce que le compilateur impose déjà
//!
//! En Python, la sérialisation des accès à une même base est une **convention**
//! : un `RLock` par chemin de fichier, qu'il faut penser à prendre. Le
//! commentaire du code d'origine explique pourquoi il existe — plusieurs fils
//! ouvrant et fermant des connexions WAL sur la même base provoquaient une
//! violation d'accès au point de contrôle WAL déclenché par `close()`.
//!
//! En Rust, `rusqlite::Connection` n'est ni `Sync` ni partageable entre fils
//! sans protection : la règle R3 du cahier des charges (« une seule écriture à
//! la fois par base ») cesse d'être une discipline pour devenir une contrainte
//! de type. Reste à sérialiser les accès de connexions *distinctes* à un même
//! fichier — c'est le rôle de [`Bases`], qui reproduit le registre de verrous
//! du Python : un verrou par fichier, les bases différentes restant parallèles.
//!
//! # Une portée, une transaction
//!
//! [`Bases::avec_base`] reproduit le comportement du gestionnaire de contexte
//! Python : `commit` si la fermeture réussit, `rollback` si elle échoue. Le
//! compilateur garantit ici ce que le `try/except/finally` garantissait à la
//! main.
//!
//! # PRAGMA appliqués à chaque ouverture
//!
//! Repris tels quels, y compris leur tolérance à l'échec — sur un support en
//! lecture seule ou un partage réseau, WAL peut être refusé sans que ce soit
//! bloquant :
//!
//! | PRAGMA | Valeur | Raison |
//! |---|---|---|
//! | `busy_timeout` | `5000` | attendre 5 s la levée d'un verrou plutôt qu'échouer |
//! | `journal_mode` | `WAL` | lecteurs et écrivain ne se bloquent plus |
//! | `synchronous` | `NORMAL` | combiné recommandé avec WAL |
//! | `foreign_keys` | **`OFF`** | cf. ci-dessous |
//!
//! # Les clés étrangères sont désactivées — et c'est voulu
//!
//! Le schéma déclare des clés étrangères (`set_prints` vers `sets`, `cards`,
//! `card_images`…), mais elles ne sont **pas appliquées** en V1.0.4 : SQLite
//! les laisse inactives par défaut, et le module `sqlite3` de Python ne les
//! active pas.
//!
//! Ce n'est pas un détail de configuration. Les données YGOJSON contiennent de
//! vraies violations d'intégrité — des tirages qui référencent une carte
//! absente de `cards.json`. La V1.0.4 les insère sans broncher ; elles sont
//! donc dans la base que l'utilisateur possède aujourd'hui, et dans les
//! classeurs qui en dérivent.
//!
//! Or `rusqlite` compilé avec la caractéristique `bundled` construit SQLite
//! avec `SQLITE_DEFAULT_FOREIGN_KEYS=1` : les contraintes sont actives. Sans le
//! `PRAGMA` ci-dessous, l'initialisation **échoue** sur la première ligne
//! fautive — ce qu'elle a fait au premier essai réel du pipeline.
//!
//! Deux réponses possibles. Filtrer les lignes fautives donnerait une base plus
//! propre, mais **différente** de celle du Python : on perdrait l'oracle, et le
//! portage cesserait d'être vérifiable. On désactive donc les contraintes, on
//! reproduit, et l'assainissement rejoint le backlog d'après-bascule.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rusqlite::{Connection, Transaction};

use crate::error::{DbError, Result};

/// Délai d'attente d'un verrou SQLite, en millisecondes.
pub const BUSY_TIMEOUT_MS: u32 = 5000;

/// Ouvre une base et applique les PRAGMA de la V1.0.4.
///
/// Le fichier — et les dossiers manquants — sont créés au besoin, comme
/// `sqlite3.connect()` suivi de `os.makedirs()`.
pub fn ouvrir(chemin: impl AsRef<Path>) -> Result<Connection> {
    let chemin = chemin.as_ref();
    if let Some(parent) = chemin.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| DbError::io(parent, e))?;
        }
    }
    let conn = Connection::open(chemin).map_err(|e| DbError::sqlite(chemin, e))?;
    appliquer_pragmas(&conn);
    Ok(conn)
}

/// Ouvre une base **en lecture seule**. Échoue si le fichier n'existe pas.
///
/// C'est le mode qu'utilise `ygo-cli` pour inspecter une installation Python :
/// on ne veut prendre aucun risque avec la base qui sert d'oracle.
pub fn ouvrir_lecture_seule(chemin: impl AsRef<Path>) -> Result<Connection> {
    let chemin = chemin.as_ref();
    let conn = Connection::open_with_flags(
        chemin,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(|e| DbError::sqlite(chemin, e))?;
    let _ = conn.busy_timeout(std::time::Duration::from_millis(u64::from(BUSY_TIMEOUT_MS)));
    Ok(conn)
}

/// Ouvre une base en mémoire, PRAGMA compris — pour les tests.
pub fn en_memoire() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    appliquer_pragmas(&conn);
    Ok(conn)
}

/// Applique les PRAGMA d'ouverture, en tolérant leur refus.
fn appliquer_pragmas(conn: &Connection) {
    let _ = conn.busy_timeout(std::time::Duration::from_millis(u64::from(BUSY_TIMEOUT_MS)));
    let _ = conn.pragma_update(None, "journal_mode", "WAL");
    let _ = conn.pragma_update(None, "synchronous", "NORMAL");
    // Iso-fonctionnel avec la V1.0.4 : voir la note du module. `rusqlite`
    // compilé en `bundled` active les clés étrangères par défaut, Python non.
    let _ = conn.pragma_update(None, "foreign_keys", "OFF");
}

/// Registre de verrous, un par fichier de base.
///
/// Portage de `centralisation_dossier._DB_LOCKS`. Deux bases différentes
/// restent accessibles en parallèle ; deux accès à la même base sont sérialisés.
///
/// Le registre est clonable et partageable entre fils : tous les clones voient
/// les mêmes verrous.
#[derive(Debug, Default, Clone)]
pub struct Bases {
    verrous: Arc<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>>,
}

impl Bases {
    /// Crée un registre vide.
    pub fn new() -> Self {
        Self::default()
    }

    /// Exécute une opération **en écriture** sur une base, sous verrou et dans
    /// une transaction unique.
    ///
    /// Équivalent de `with sqlite_ctx(db_path) as conn:` :
    ///
    /// - le verrou de ce fichier est pris pour toute la durée de l'appel ;
    /// - la base est ouverte, PRAGMA appliqués ;
    /// - une transaction est ouverte ;
    /// - elle est validée si la fermeture retourne `Ok`, annulée sinon ;
    /// - la connexion est refermée à la sortie, quoi qu'il arrive.
    ///
    /// ```
    /// # fn main() -> ygo_db::Result<()> {
    /// use ygo_db::connexion::Bases;
    /// use ygo_db::schema::{creer_tout, DDL_CLASSEUR};
    ///
    /// let dossier = std::env::temp_dir().join("ygo-doctest");
    /// std::fs::create_dir_all(&dossier).ok();
    /// let base = dossier.join("RA05.db");
    /// let _ = std::fs::remove_file(&base);
    ///
    /// let bases = Bases::new();
    /// bases.avec_base(&base, |tx| {
    ///     creer_tout(tx, DDL_CLASSEUR)?;
    ///     tx.execute("INSERT INTO meta (key, value) VALUES ('cols', '3')", [])?;
    ///     Ok(())
    /// })?;
    ///
    /// let n: i64 = bases.avec_base_lecture(&base, |c| {
    ///     Ok(c.query_row("SELECT count(*) FROM meta", [], |r| r.get(0))?)
    /// })?;
    /// assert_eq!(n, 1);
    /// # let _ = std::fs::remove_file(&base);
    /// # Ok(())
    /// # }
    /// ```
    pub fn avec_base<T, F>(&self, chemin: impl AsRef<Path>, operation: F) -> Result<T>
    where
        F: FnOnce(&Transaction<'_>) -> Result<T>,
    {
        let chemin = chemin.as_ref();
        let verrou = self.verrou_de(chemin)?;
        let _garde = verrou
            .lock()
            .map_err(|_| DbError::VerrouEmpoisonne(chemin.to_path_buf()))?;

        let mut conn = ouvrir(chemin)?;
        let tx = conn.transaction().map_err(|e| DbError::sqlite(chemin, e))?;
        match operation(&tx) {
            Ok(valeur) => {
                tx.commit().map_err(|e| DbError::sqlite(chemin, e))?;
                Ok(valeur)
            }
            Err(e) => {
                // `Transaction` annule d'elle-même à sa destruction ; on est
                // explicite pour que l'intention soit lisible.
                let _ = tx.rollback();
                Err(e)
            }
        }
    }

    /// Exécute une opération **en lecture** sur une base, sous verrou, sans
    /// ouvrir de transaction.
    pub fn avec_base_lecture<T, F>(&self, chemin: impl AsRef<Path>, operation: F) -> Result<T>
    where
        F: FnOnce(&Connection) -> Result<T>,
    {
        let chemin = chemin.as_ref();
        let verrou = self.verrou_de(chemin)?;
        let _garde = verrou
            .lock()
            .map_err(|_| DbError::VerrouEmpoisonne(chemin.to_path_buf()))?;

        let conn = ouvrir(chemin)?;
        operation(&conn)
    }

    /// Nombre de fichiers actuellement suivis — diagnostic.
    pub fn nb_bases_suivies(&self) -> usize {
        self.verrous.lock().map(|r| r.len()).unwrap_or(0)
    }

    fn verrou_de(&self, chemin: &Path) -> Result<Arc<Mutex<()>>> {
        let cle = normaliser_chemin(chemin);
        let mut registre = self
            .verrous
            .lock()
            .map_err(|_| DbError::VerrouEmpoisonne(cle.clone()))?;
        Ok(Arc::clone(registre.entry(cle).or_default()))
    }
}

/// Normalise un chemin pour servir de clé de verrou.
///
/// Équivalent de `os.path.normcase(os.path.abspath(path))` : sous Windows la
/// casse n'est pas significative — `BDD/Cardinfo.DB` et `bdd/cardinfo.db`
/// désignent le même fichier et doivent partager le même verrou.
///
/// `canonicalize` échoue si le fichier n'existe pas encore ; on retombe alors
/// sur une simple absolutisation, ce qui est le cas d'une base sur le point
/// d'être créée.
fn normaliser_chemin(chemin: &Path) -> PathBuf {
    let absolu = chemin
        .canonicalize()
        .or_else(|_| std::path::absolute(chemin))
        .unwrap_or_else(|_| chemin.to_path_buf());

    if cfg!(windows) {
        PathBuf::from(absolu.to_string_lossy().to_lowercase())
    } else {
        absolu
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;
    use crate::schema::{creer_tout, DDL_CLASSEUR};

    #[test]
    fn ouvrir_cree_les_dossiers_intermediaires() {
        let tmp = tempfile::tempdir().unwrap();
        let chemin = tmp
            .path()
            .join("bdd")
            .join("classeur_creer")
            .join("RA05")
            .join("RA05.db");

        let conn = ouvrir(&chemin).unwrap();
        creer_tout(&conn, DDL_CLASSEUR).unwrap();
        drop(conn);

        assert!(chemin.is_file());
    }

    #[test]
    fn les_pragmas_sont_appliques() {
        let tmp = tempfile::tempdir().unwrap();
        let conn = ouvrir(tmp.path().join("test.db")).unwrap();

        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode.to_lowercase(), "wal");

        let sync: i64 = conn
            .query_row("PRAGMA synchronous", [], |r| r.get(0))
            .unwrap();
        assert_eq!(sync, 1, "NORMAL");
    }

    /// Iso-fonctionnel avec la V1.0.4 : les données YGOJSON contiennent de
    /// vraies violations d'intégrité référentielle, que le Python insère sans
    /// broncher. Avec `rusqlite` en `bundled`, les contraintes sont actives par
    /// défaut et l'initialisation échouerait — d'où le PRAGMA.
    #[test]
    fn les_cles_etrangeres_sont_desactivees() {
        let tmp = tempfile::tempdir().unwrap();
        let conn = ouvrir(tmp.path().join("fk.db")).unwrap();

        let actif: i64 = conn
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .unwrap();
        assert_eq!(actif, 0, "les clés étrangères doivent rester inactives");

        // Un tirage qui référence une carte absente doit être accepté, comme
        // en V1.0.4.
        conn.execute("CREATE TABLE parent (id TEXT PRIMARY KEY)", [])
            .unwrap();
        conn.execute(
            "CREATE TABLE enfant (id INTEGER PRIMARY KEY, parent_id TEXT,              FOREIGN KEY (parent_id) REFERENCES parent(id))",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO enfant (parent_id) VALUES ('absent')", [])
            .unwrap();
    }

    #[test]
    fn lecture_seule_refuse_l_ecriture() {
        let tmp = tempfile::tempdir().unwrap();
        let chemin = tmp.path().join("lecture.db");
        {
            let conn = ouvrir(&chemin).unwrap();
            creer_tout(&conn, DDL_CLASSEUR).unwrap();
        }

        let conn = ouvrir_lecture_seule(&chemin).unwrap();
        assert!(conn
            .execute("INSERT INTO meta (key, value) VALUES ('a', 'b')", [])
            .is_err());
    }

    #[test]
    fn une_erreur_annule_toute_la_transaction() {
        let tmp = tempfile::tempdir().unwrap();
        let chemin = tmp.path().join("transaction.db");
        let bases = Bases::new();

        bases
            .avec_base(&chemin, |tx| creer_tout(tx, DDL_CLASSEUR))
            .unwrap();

        let resultat: Result<()> = bases.avec_base(&chemin, |tx| {
            tx.execute("INSERT INTO meta (key, value) VALUES ('a', '1')", [])?;
            tx.execute("INSERT INTO meta (key, value) VALUES ('b', '2')", [])?;
            // Échec après deux insertions réussies.
            Err(DbError::SchemaNonConforme(1))
        });
        assert!(resultat.is_err());

        let n: i64 = bases
            .avec_base_lecture(&chemin, |c| {
                Ok(c.query_row("SELECT count(*) FROM meta", [], |r| r.get(0))?)
            })
            .unwrap();
        assert_eq!(n, 0, "aucune des deux insertions ne doit survivre");
    }

    #[test]
    fn deux_bases_differentes_ont_deux_verrous() {
        let tmp = tempfile::tempdir().unwrap();
        let bases = Bases::new();

        bases
            .avec_base(tmp.path().join("a.db"), |_| Ok(()))
            .unwrap();
        bases
            .avec_base(tmp.path().join("b.db"), |_| Ok(()))
            .unwrap();

        assert_eq!(bases.nb_bases_suivies(), 2);
    }

    #[test]
    fn le_verrou_serialise_les_acces_a_une_meme_base() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let tmp = tempfile::tempdir().unwrap();
        let chemin = tmp.path().join("partagee.db");
        let bases = Bases::new();
        bases.avec_base(&chemin, |_| Ok(())).unwrap();

        let simultanes = AtomicUsize::new(0);
        let maximum = AtomicUsize::new(0);

        std::thread::scope(|s| {
            for _ in 0..8 {
                let bases = bases.clone();
                let chemin = chemin.clone();
                let simultanes = &simultanes;
                let maximum = &maximum;
                s.spawn(move || {
                    for _ in 0..10 {
                        let _ = bases.avec_base_lecture(&chemin, |_| {
                            let n = simultanes.fetch_add(1, Ordering::SeqCst) + 1;
                            maximum.fetch_max(n, Ordering::SeqCst);
                            std::thread::yield_now();
                            simultanes.fetch_sub(1, Ordering::SeqCst);
                            Ok(())
                        });
                    }
                });
            }
        });

        assert_eq!(
            maximum.load(Ordering::SeqCst),
            1,
            "un seul fil à la fois doit détenir le verrou d'une base"
        );
    }
}
