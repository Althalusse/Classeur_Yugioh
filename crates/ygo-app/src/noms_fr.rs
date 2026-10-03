// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Les noms français des classeurs, remis à jour depuis la base.
//!
//! # Le constat — 2026-10-02
//!
//! Un classeur reçoit les noms FR de `cardinfo.db` **à sa création**, et plus
//! jamais ensuite. Mesuré sur les 16 classeurs de l'utilisateur :
//!
//! | | lignes |
//! |---|---:|
//! | sans nom FR | 1 295 |
//! | … dont la base **a** le nom officiel | **1 067** (MP25 373, MP24 316, RA05 278…) |
//! | … dont **aucune source** n'a de nom FR | 228 (cartes OCG de LOCH-JP / LOCR-JP, 2 de MP25) |
//! | nom FR **différent** de la base | 4 (« Inifini Éphémère » pour « Infini Éphémère ») |
//!
//! La plupart de ces classeurs n'ont pas de `card_uuid` : la carte se retrouve
//! par son identifiant d'image YGOPRODeck, puis par son identifiant de carte,
//! puis par son nom anglais **exact et unique** ([`Index::nom_fr`]).
//!
//! # La règle de l'utilisateur : on n'invente pas
//!
//! Un nom FR n'est posé que si la base en a un — le nom officiel que YGOJSON
//! tient de Konami. Sans nom FR à la source, la ligne reste vide (l'affichage
//! se replie sur l'anglais, il n'écrit rien). Un nom FR **existant** n'est
//! jamais effacé faute de source ; il n'est remplacé que par le nom officiel
//! qui en diffère — c'est ce qui corrige les quatre « Inifini ».
//!
//! L'ancien cache Python (`fr_names_cache.json`, 13 677 noms YGOPRODeck) a été
//! confronté aux 1 295 lignes : il n'en couvre **aucune** que la base ne
//! couvre déjà. Il n'est pas porté.
//!
//! # Quand
//!
//! À chaque mise à jour de la base ([`crate::maj::mettre_a_jour`]) — c'est le
//! seul événement qui puisse apporter un nom — et à la demande
//! (`ygo-cli noms-fr`).

use std::collections::HashMap;

use rusqlite::Connection;
use ygo_core::paths::Paths;

use crate::error::Result;

/// Ce que la base sait des noms français, indexé pour retrouver une carte
/// par tous les moyens qu'un classeur peut offrir.
#[derive(Debug, Clone, Default)]
pub struct Index {
    /// `card_uuid` → nom FR officiel.
    fr: HashMap<String, String>,
    /// Identifiant d'image YGOPRODeck → `card_uuid`.
    par_image: HashMap<i64, String>,
    /// Identifiant de carte YGOPRODeck → `card_uuid`.
    par_id: HashMap<i64, String>,
    /// Nom anglais → `card_uuid`, `None` quand plusieurs cartes le portent.
    par_nom: HashMap<String, Option<String>>,
}

impl Index {
    /// Lit `cardinfo.db`.
    ///
    /// # Errors
    ///
    /// Rend une erreur si la base est illisible.
    pub fn charger(cardinfo: &Connection) -> Result<Self> {
        let mut index = Self::default();
        let mut r = cardinfo.prepare(
            "SELECT card_uuid, name FROM card_texts
              WHERE language = 'fr' AND COALESCE(TRIM(name), '') <> ''",
        )?;
        for l in r.query_map([], |l| Ok((l.get::<_, String>(0)?, l.get::<_, String>(1)?)))? {
            let (uuid, nom) = l?;
            index.fr.insert(uuid, nom.trim().to_owned());
        }
        let mut r = cardinfo.prepare(
            "SELECT ygoprodeck_image_id, card_uuid FROM card_images
              WHERE ygoprodeck_image_id IS NOT NULL",
        )?;
        for l in r.query_map([], |l| Ok((l.get::<_, i64>(0)?, l.get::<_, String>(1)?)))? {
            let (id, uuid) = l?;
            index.par_image.insert(id, uuid);
        }
        let mut r = cardinfo
            .prepare("SELECT ygoprodeck_id, uuid FROM cards WHERE ygoprodeck_id IS NOT NULL")?;
        for l in r.query_map([], |l| Ok((l.get::<_, i64>(0)?, l.get::<_, String>(1)?)))? {
            let (id, uuid) = l?;
            index.par_id.insert(id, uuid);
        }
        let mut r = cardinfo.prepare(
            "SELECT name, card_uuid FROM card_texts
              WHERE language = 'en' AND COALESCE(TRIM(name), '') <> ''",
        )?;
        for l in r.query_map([], |l| Ok((l.get::<_, String>(0)?, l.get::<_, String>(1)?)))? {
            let (nom, uuid) = l?;
            index
                .par_nom
                .entry(nom.trim().to_owned())
                .and_modify(|e| {
                    if e.as_deref() != Some(uuid.as_str()) {
                        *e = None;
                    }
                })
                .or_insert(Some(uuid));
        }
        Ok(index)
    }

    /// La carte d'une ligne de classeur : par `card_uuid`, puis par
    /// identifiant d'image, puis d'identifiant de carte, puis par nom anglais
    /// exact — s'il n'est porté que par une seule carte.
    #[must_use]
    pub fn carte<'a>(
        &'a self,
        card_uuid: Option<&'a str>,
        card_image_id: Option<i64>,
        nom: &str,
    ) -> Option<&'a str> {
        if let Some(u) = card_uuid.map(str::trim).filter(|u| !u.is_empty()) {
            return Some(u);
        }
        if let Some(id) = card_image_id.filter(|i| *i > 0) {
            if let Some(u) = self.par_image.get(&id).or_else(|| self.par_id.get(&id)) {
                return Some(u);
            }
        }
        self.par_nom.get(nom.trim()).and_then(Option::as_deref)
    }

    /// Le nom FR officiel d'une ligne — `None` si la carte est introuvable
    /// ou si la base n'a pas de nom FR pour elle.
    #[must_use]
    pub fn nom_fr(
        &self,
        card_uuid: Option<&str>,
        card_image_id: Option<i64>,
        nom: &str,
    ) -> Option<&str> {
        self.carte(card_uuid, card_image_id, nom)
            .and_then(|u| self.fr.get(u))
            .map(String::as_str)
    }
}

/// Ce qu'un classeur gagnerait.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rapport {
    /// Lignes lues.
    pub lues: usize,
    /// `(rowid, nom FR)` à écrire.
    pub a_poser: Vec<(i64, String)>,
    /// Lignes sans nom FR qui en reçoivent un.
    pub ajoutes: usize,
    /// Lignes dont le nom FR diffère du nom officiel, et qui le reçoivent.
    pub corriges: usize,
    /// Lignes sans nom FR dont la carte n'en a pas à la source — elles
    /// restent vides.
    pub sans_source: usize,
    /// Lignes sans nom FR dont la carte n'a pas été retrouvée.
    pub introuvables: usize,
}

/// Une ligne de classeur telle que l'analyse la lit : `rowid`, `card_uuid`,
/// identifiant d'image, nom anglais, nom FR actuel.
type LigneClasseur = (i64, Option<String>, Option<i64>, String, String);

/// Compare un classeur à la base, sans rien écrire.
///
/// Un classeur sans colonne `name_fr` rend un rapport vide : rien n'ajoute les
/// colonnes au démarrage, et ce n'est pas à ce module de le faire.
///
/// # Errors
///
/// Rend une erreur si le classeur est illisible.
pub fn analyser(classeur: &Connection, index: &Index) -> Result<Rapport> {
    let colonnes = ygo_db::migrations::colonnes(classeur, "cards")?;
    let a = |c: &str| colonnes.iter().any(|x| x == c);
    if !a("name_fr") {
        return Ok(Rapport::default());
    }
    let uuid = if a("card_uuid") { "card_uuid" } else { "NULL" };
    let image = if a("card_image_id") {
        "card_image_id"
    } else {
        "NULL"
    };
    // Les colonnes viennent d'ici, jamais d'une donnée.
    let requete = format!(
        "SELECT rowid, {uuid}, {image}, COALESCE(name, ''), COALESCE(TRIM(name_fr), '') FROM cards"
    );
    let mut r = classeur.prepare(&requete)?;
    let lignes: Vec<LigneClasseur> = r
        .query_map([], |l| {
            Ok((l.get(0)?, l.get(1)?, l.get(2)?, l.get(3)?, l.get(4)?))
        })?
        .collect::<std::result::Result<_, _>>()?;
    let mut rapport = Rapport {
        lues: lignes.len(),
        ..Rapport::default()
    };
    for (rowid, uuid, image, nom, actuel) in lignes {
        let officiel = index.nom_fr(uuid.as_deref(), image, &nom);
        match officiel {
            Some(o) if actuel.is_empty() => {
                rapport.ajoutes += 1;
                rapport.a_poser.push((rowid, o.to_owned()));
            }
            Some(o) if o != actuel => {
                rapport.corriges += 1;
                rapport.a_poser.push((rowid, o.to_owned()));
            }
            Some(_) => {}
            None if actuel.is_empty() => {
                if index.carte(uuid.as_deref(), image, &nom).is_some() {
                    rapport.sans_source += 1;
                } else {
                    rapport.introuvables += 1;
                }
            }
            // Un nom existant sans source n'est jamais effacé.
            None => {}
        }
    }
    Ok(rapport)
}

/// Écrit ce que l'analyse a trouvé, en une transaction.
///
/// # Errors
///
/// Rend une erreur si le classeur ne peut pas être écrit.
pub fn appliquer(classeur: &mut Connection, rapport: &Rapport) -> Result<usize> {
    let tx = classeur.transaction()?;
    let mut n = 0;
    {
        let mut r = tx.prepare("UPDATE cards SET name_fr = ?1 WHERE rowid = ?2")?;
        for (rowid, nom) in &rapport.a_poser {
            n += r.execute(rusqlite::params![nom, rowid])?;
        }
    }
    tx.commit()?;
    Ok(n)
}

/// Le total d'un rafraîchissement.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Bilan {
    /// Classeurs relus.
    pub classeurs: usize,
    /// Noms FR posés là où il n'y en avait pas.
    pub ajoutes: usize,
    /// Noms FR remplacés par le nom officiel.
    pub corriges: usize,
    /// Lignes restées sans nom FR faute de source.
    pub sans_source: usize,
}

/// Rafraîchit les noms FR de **tous** les classeurs — l'étape d'après mise à
/// jour. Un classeur illisible est journalisé et sauté.
///
/// # Errors
///
/// Rend une erreur si `cardinfo.db` est illisible.
pub fn rafraichir(paths: &Paths) -> Result<Bilan> {
    let cardinfo = ygo_db::connexion::ouvrir_lecture_seule(paths.cardinfo_db())?;
    let index = Index::charger(&cardinfo)?;
    let mut bilan = Bilan::default();
    for code in paths.classeurs_existants() {
        let resultat = Connection::open(paths.classeur_db(&code))
            .map_err(crate::error::AppError::from)
            .and_then(|mut conn| {
                let r = analyser(&conn, &index)?;
                if !r.a_poser.is_empty() {
                    appliquer(&mut conn, &r)?;
                }
                Ok(r)
            });
        match resultat {
            Ok(r) => {
                bilan.classeurs += 1;
                bilan.ajoutes += r.ajoutes;
                bilan.corriges += r.corriges;
                bilan.sans_source += r.sans_source;
            }
            Err(e) => tracing::warn!(classeur = %code, erreur = %e, "noms FR non rafraîchis"),
        }
    }
    Ok(bilan)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    /// Une base réduite : Infinite Impermanence (FR officiel), une carte OCG
    /// sans nom FR, et deux cartes qui portent le même nom anglais.
    fn cardinfo() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE cards (uuid TEXT, ygoprodeck_id INTEGER);
             CREATE TABLE card_images (card_uuid TEXT, ygoprodeck_image_id INTEGER);
             CREATE TABLE card_texts (card_uuid TEXT, language TEXT, name TEXT);
             INSERT INTO cards VALUES ('u-infini', 10045474), ('u-ocg', NULL),
                                      ('u-a', 1), ('u-b', 2);
             INSERT INTO card_images VALUES ('u-infini', 10045474), ('u-infini', 10045475);
             INSERT INTO card_texts VALUES
               ('u-infini', 'en', 'Infinite Impermanence'),
               ('u-infini', 'fr', 'Infini Éphémère'),
               ('u-ocg', 'en', 'Multiplying Kuriboh!'),
               ('u-a', 'en', 'Homonyme'), ('u-a', 'fr', 'Homonyme A'),
               ('u-b', 'en', 'Homonyme'), ('u-b', 'fr', 'Homonyme B');",
        )
        .unwrap();
        c
    }

    /// `(card_uuid, card_image_id, name, name_fr)`.
    type Ligne<'a> = (Option<&'a str>, Option<i64>, &'a str, Option<&'a str>);

    fn classeur(lignes: &[Ligne<'_>]) -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE cards (card_uuid TEXT, card_image_id INTEGER, name TEXT, name_fr TEXT);",
        )
        .unwrap();
        for (u, i, n, f) in lignes {
            c.execute(
                "INSERT INTO cards VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![u, i, n, f],
            )
            .unwrap();
        }
        c
    }

    fn noms(c: &Connection) -> Vec<Option<String>> {
        let mut r = c
            .prepare("SELECT name_fr FROM cards ORDER BY rowid")
            .unwrap();
        r.query_map([], |l| l.get(0))
            .unwrap()
            .map(std::result::Result::unwrap)
            .collect()
    }

    #[test]
    fn la_carte_se_retrouve_par_tous_les_chemins() {
        let index = Index::charger(&cardinfo()).unwrap();
        // Par card_uuid, par image alternative, par identifiant, par nom.
        assert_eq!(
            index.nom_fr(Some("u-infini"), None, ""),
            Some("Infini Éphémère")
        );
        assert_eq!(
            index.nom_fr(None, Some(10045475), ""),
            Some("Infini Éphémère")
        );
        assert_eq!(
            index.nom_fr(None, None, "Infinite Impermanence"),
            Some("Infini Éphémère")
        );
        // Un nom porté par deux cartes ne désigne personne.
        assert_eq!(index.nom_fr(None, None, "Homonyme"), None);
        // Un identifiant négatif est un artwork Yugipedia haché : pas une image YGOPRODeck.
        assert_eq!(
            index.nom_fr(None, Some(-10045474), "Infinite Impermanence"),
            Some("Infini Éphémère")
        );
    }

    /// La règle de l'utilisateur : poser l'officiel, corriger ce qui en
    /// diffère, ne rien inventer, ne rien effacer.
    #[test]
    fn on_pose_on_corrige_on_n_invente_rien() {
        let index = Index::charger(&cardinfo()).unwrap();
        let mut c = classeur(&[
            (None, Some(10045474), "Infinite Impermanence", None),
            (
                None,
                None,
                "Infinite Impermanence",
                Some("Inifini Éphémère"),
            ),
            (Some("u-ocg"), None, "Multiplying Kuriboh!", None),
            (None, None, "Carte inconnue", None),
            (
                Some("u-ocg"),
                None,
                "Multiplying Kuriboh!",
                Some("Nom saisi"),
            ),
            (None, None, "Homonyme", None),
        ]);
        let r = analyser(&c, &index).unwrap();
        assert_eq!(
            (r.ajoutes, r.corriges, r.sans_source, r.introuvables),
            (1, 1, 1, 2)
        );
        appliquer(&mut c, &r).unwrap();
        assert_eq!(
            noms(&c),
            vec![
                Some("Infini Éphémère".into()),
                Some("Infini Éphémère".into()),
                None,
                None,
                Some("Nom saisi".into()),
                None,
            ]
        );
        // Une seconde passe ne trouve plus rien.
        assert!(analyser(&c, &index).unwrap().a_poser.is_empty());
    }

    #[test]
    fn un_classeur_sans_colonne_name_fr_est_laisse_tel_quel() {
        let index = Index::charger(&cardinfo()).unwrap();
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE cards (name TEXT); INSERT INTO cards VALUES ('X');")
            .unwrap();
        assert_eq!(analyser(&c, &index).unwrap(), Rapport::default());
    }
}
