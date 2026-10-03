// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Les exemplaires d'une carte : un état et une édition **chacun**.
//!
//! # Le besoin — 2026-10-01
//!
//! Une ligne de classeur est un tirage : un numéro, une rareté, une
//! illustration. Elle porte **une** quantité, **un** état et **une** édition.
//! Cinq exemplaires d'un même tirage, dont deux abîmés, ne se décrivaient
//! donc pas : changer l'état les changeait tous les cinq. L'import le
//! signalait déjà — deux lignes CSV d'états différents fusionnées sur une
//! même ligne, la première l'emportant.
//!
//! Demande de l'utilisateur : la carte une fois, **dépliable** en ses
//! exemplaires ; par défaut tous identiques, chacun modifiable.
//!
//! # Le modèle : un état commun, et des exemplaires « à part »
//!
//! La ligne garde ce qu'elle avait : `quantite`, `qualite`, `edition`. Ces
//! deux dernières deviennent l'état **commun** — celui de tout exemplaire
//! qu'on n'a pas réglé autrement. La table `exemplaires` ne contient que les
//! exemplaires **à part** :
//!
//! ```text
//! exemplaires (id, card_rowid, qualite, edition)
//! ```
//!
//! Cinq exemplaires dont deux en `PL` : `quantite = 5`, `qualite = NM`, et
//! deux lignes `PL` dans `exemplaires`. Les trois autres sont **standard**.
//!
//! Pourquoi pas une ligne par exemplaire : parce que tout le reste du
//! projet — statistiques, playset, possession, filtres, l'application Python
//! elle-même — lit `quantite` et rien d'autre. Ce modèle ne le dérange pas :
//! un classeur sans table `exemplaires`, ou dont la table est vide, se lit
//! **exactement** comme avant. Rien à migrer.
//!
//! # Deux invariants, tenus par chaque écriture de ce module
//!
//! 1. Un exemplaire à part **diffère** de l'état commun — sinon il est
//!    standard, et sa ligne est effacée ([`normaliser`]).
//! 2. Il y a au plus `quantite` exemplaires à part.
//!
//! La lecture ne s'y fie pas pour autant ([`Exemplaires::depuis`]) : un
//! classeur écrit par une autre version, ou une écriture interrompue, ne doit
//! pas faire apparaître six exemplaires sur une carte qui en compte cinq.
//!
//! # Qui part quand la quantité baisse
//!
//! Décision de l'utilisateur : **un exemplaire standard d'abord**. Celui
//! qu'on a pris la peine de régler à part est celui qu'on veut garder ; ce
//! n'est que lorsqu'il ne reste plus de standard que le dernier réglé part
//! ([`Exemplaires::a_effacer`]). Pour en retirer un précis, la ligne dépliée
//! a son propre bouton ([`Demande::Retirer`]).
//!
//! # Ce qu'un exemplaire reçoit en entrant dans la collection — 2026-10-01
//!
//! Décision de l'utilisateur : une carte qui devient possédée sans état
//! reçoit **Mint** ([`QUALITE_PAR_DEFAUT`]) — il ouvre ses boosters lui-même.
//! Son édition est celle que la base de cartes connaît pour ce tirage, **quand
//! elle n'en connaît qu'une** : `RA02` n'a existé qu'en 1st, `LCKC` qu'en
//! Unlimited ; `LOB` a eu les deux, et là on ne devine pas
//! ([`Defauts::charger`]).
//!
//! Rien de ce qui est déjà renseigné n'est écrasé : sur l'installation réelle,
//! `LDK2` porte « 1st » (venu de Scanflip) là où la base dit « Limited ». La
//! base sait ce qui a été imprimé, l'utilisateur sait ce qu'il a en main.
//!
//! # La table naît à la première écriture
//!
//! La migration des colonnes (`ygo_db::migrations`) n'est appelée nulle part
//! en production. Plutôt que d'ajouter un passage au démarrage sur chaque
//! classeur, la table est créée par la **première** écriture qui en a besoin
//! ([`creer_table`]), et toute lecture tolère son absence.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension};

use ygo_core::paths::Paths;

use crate::error::{AppError, Result};

/// L'état d'un exemplaire qui entre dans la collection sans en avoir —
/// décision de l'utilisateur du 2026-10-01.
pub const QUALITE_PAR_DEFAUT: &str = "M";

/// Ce qu'un exemplaire reçoit quand il entre dans la collection.
///
/// `Defauts::default()` ne pose **rien** : c'est ce qu'emploient les tests et
/// tout appelant qui ne veut pas de remplissage.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Defauts {
    /// La qualité posée sur une ligne qui n'en a pas — vide : aucune.
    pub qualite: String,
    /// L'édition de chaque `set_code` dont la base de cartes ne connaît
    /// qu'**une** édition.
    pub editions: HashMap<String, String>,
}

impl Defauts {
    /// L'édition connue d'un tirage, s'il n'en a qu'une.
    #[must_use]
    pub fn edition(&self, set_code: &str) -> Option<&str> {
        self.editions.get(set_code.trim()).map(String::as_str)
    }

    /// Les défauts d'un classeur : Mint, et les éditions que `cardinfo.db`
    /// connaît pour ses tirages.
    ///
    /// Ne lève jamais : sans base de cartes lisible, seule la qualité est
    /// posée — une édition qu'on ne peut pas vérifier ne s'invente pas.
    #[must_use]
    pub fn charger(paths: &Paths, classeur: &Connection) -> Self {
        let editions = ygo_db::connexion::ouvrir_lecture_seule(paths.cardinfo_db())
            .ok()
            .and_then(|cardinfo| editions_connues(&cardinfo, classeur).ok())
            .unwrap_or_default();
        Self {
            qualite: QUALITE_PAR_DEFAUT.to_owned(),
            editions,
        }
    }
}

/// L'édition de chaque tirage d'un classeur, quand `cardinfo.db` n'en connaît
/// qu'une.
///
/// Mesuré le 2026-10-01 : sur les 309 263 tirages `(set_code, rareté)` de la
/// base, 600 ont plusieurs éditions — les vieux sets (`LOB`…) imprimés en
/// 1st puis en Unlimited. Tous les tirages des 16 classeurs de l'utilisateur
/// n'en ont qu'une.
///
/// # Errors
///
/// Rend une erreur si l'une des deux bases est illisible.
pub fn editions_connues(
    cardinfo: &Connection,
    classeur: &Connection,
) -> Result<HashMap<String, String>> {
    let mut codes = classeur.prepare(
        "SELECT DISTINCT TRIM(set_code) FROM cards WHERE COALESCE(TRIM(set_code), '') <> ''",
    )?;
    let codes: Vec<String> = codes
        .query_map([], |l| l.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    let mut editions = cardinfo.prepare(
        "SELECT DISTINCT edition FROM set_prints
          WHERE set_code = ?1 AND COALESCE(TRIM(edition), '') <> ''",
    )?;
    let mut connues = HashMap::new();
    for code in codes {
        let trouvees: Vec<String> = editions
            .query_map([&code], |l| l.get(0))?
            .collect::<std::result::Result<_, _>>()?;
        if let [seule] = trouvees.as_slice() {
            connues.insert(code, seule.trim().to_owned());
        }
    }
    Ok(connues)
}

/// Pose les défauts sur une ligne : la qualité si elle n'en a pas,
/// l'édition si elle n'en a pas et que la base n'en connaît qu'une. Ce qui
/// est déjà renseigné n'est **jamais** touché.
///
/// Rend `(qualité posée, édition posée)`.
///
/// # Errors
///
/// Rend une erreur si la base est inaccessible ou en lecture seule.
pub fn completer(conn: &Connection, rowid: i64, defauts: &Defauts) -> Result<(bool, bool)> {
    let colonnes = ygo_db::migrations::colonnes(conn, "cards")?;
    let a = |nom: &str| colonnes.iter().any(|c| c == nom);
    let mut qualite = false;
    if !defauts.qualite.is_empty() && a("qualite") {
        qualite = conn.execute(
            "UPDATE cards SET qualite = ?1
              WHERE rowid = ?2 AND COALESCE(TRIM(qualite), '') = ''",
            rusqlite::params![defauts.qualite, rowid],
        )? > 0;
    }
    let mut edition = false;
    if !defauts.editions.is_empty() && a("edition") {
        let code: Option<String> = conn
            .query_row(
                "SELECT COALESCE(TRIM(set_code), '') FROM cards WHERE rowid = ?1",
                [rowid],
                |l| l.get(0),
            )
            .optional()?;
        if let Some(connue) = code.as_deref().and_then(|c| defauts.edition(c)) {
            edition = conn.execute(
                "UPDATE cards SET edition = ?1
                  WHERE rowid = ?2 AND COALESCE(TRIM(edition), '') = ''",
                rusqlite::params![connue, rowid],
            )? > 0;
        }
    }
    if qualite || edition {
        // L'état commun a pu rejoindre celui d'un exemplaire à part.
        normaliser(conn, rowid)?;
    }
    Ok((qualite, edition))
}

/// Ce que [`completer_classeur`] a trouvé — ou posé.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Completion {
    /// Lignes possédées lues.
    pub possedees: usize,
    /// Lignes qui reçoivent la qualité par défaut.
    pub qualites: usize,
    /// Lignes qui reçoivent l'édition connue.
    pub editions: usize,
    /// Lignes sans édition dont la base ne connaît pas l'édition unique —
    /// elles restent vides.
    pub editions_inconnues: usize,
}

/// Pose les défauts sur **toutes** les lignes possédées d'un classeur qui
/// en manquent — la reprise de l'existant. Avec `ecrire = false`, compte
/// sans rien écrire.
///
/// # Errors
///
/// Rend une erreur si la base est inaccessible, ou en lecture seule quand
/// `ecrire` est vrai.
pub fn completer_classeur(
    conn: &mut Connection,
    defauts: &Defauts,
    ecrire: bool,
) -> Result<Completion> {
    let colonnes = ygo_db::migrations::colonnes(conn, "cards")?;
    let a = |nom: &str| colonnes.iter().any(|c| c == nom);
    let qualite = if a("qualite") {
        "COALESCE(TRIM(qualite), '')"
    } else {
        "''"
    };
    let edition = if a("edition") {
        "COALESCE(TRIM(edition), '')"
    } else {
        "''"
    };
    // Les expressions viennent d'ici, jamais d'une donnée.
    let requete = format!(
        "SELECT rowid, {qualite}, {edition}, COALESCE(TRIM(set_code), '')
           FROM cards WHERE COALESCE(possessed, 0) = 1"
    );
    let lignes: Vec<(i64, String, String, String)> = {
        let mut r = conn.prepare(&requete)?;
        let lignes = r
            .query_map([], |l| Ok((l.get(0)?, l.get(1)?, l.get(2)?, l.get(3)?)))?
            .collect::<std::result::Result<_, _>>()?;
        lignes
    };
    let mut bilan = Completion {
        possedees: lignes.len(),
        ..Completion::default()
    };
    for (_, q, e, code) in &lignes {
        if q.is_empty() && !defauts.qualite.is_empty() && a("qualite") {
            bilan.qualites += 1;
        }
        if e.is_empty() && a("edition") {
            if defauts.edition(code).is_some() {
                bilan.editions += 1;
            } else {
                bilan.editions_inconnues += 1;
            }
        }
    }
    if ecrire {
        let tx = conn.transaction()?;
        for (rowid, ..) in &lignes {
            completer(&tx, *rowid, defauts)?;
        }
        tx.commit()?;
    }
    Ok(bilan)
}

/// L'état d'un exemplaire : sa qualité de conservation et son édition.
///
/// Chaîne vide : non renseigné. Les valeurs sont celles des colonnes de la
/// ligne — `NM`, `PL`… et `1st` / `unlimited` / `limited`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Etat {
    /// Qualité de conservation (`NM`, `PL`…).
    pub qualite: String,
    /// Édition (`1st`, `unlimited`, `limited`).
    pub edition: String,
}

impl Etat {
    /// Un état, débarrassé des espaces de bord.
    #[must_use]
    pub fn nouveau(qualite: &str, edition: &str) -> Self {
        Self {
            qualite: qualite.trim().to_owned(),
            edition: edition.trim().to_owned(),
        }
    }
}

/// Un exemplaire, désigné pour une action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Exemplaire {
    /// Un exemplaire standard — ils sont interchangeables : on n'en désigne
    /// pas un en particulier.
    Standard,
    /// Un exemplaire à part, par son identifiant dans la table.
    APart(i64),
}

/// Quel champ une action « pour tous » écrit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Champ {
    /// La qualité.
    Qualite,
    /// L'édition.
    Edition,
}

/// Les exemplaires d'une ligne de classeur.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Exemplaires {
    /// Nombre total d'exemplaires — la `quantite` de la ligne.
    pub quantite: i64,
    /// L'état commun, porté par la ligne.
    pub commun: Etat,
    /// Les exemplaires à part, par identifiant croissant : `(id, état)`.
    pub a_part: Vec<(i64, Etat)>,
}

impl Exemplaires {
    /// Assemble les exemplaires d'une ligne en **rétablissant** les
    /// invariants : un exemplaire à part identique à l'état commun redevient
    /// standard, et il n'y en a jamais plus que la quantité.
    ///
    /// ```
    /// use ygo_app::exemplaires::{Etat, Exemplaires};
    /// let nm = Etat::nouveau("NM", "1st");
    /// let pl = Etat::nouveau("PL", "1st");
    /// let ex = Exemplaires::depuis(1, nm.clone(), vec![(1, pl.clone()), (2, nm), (3, pl)]);
    /// // L'exemplaire 2 est standard ; l'exemplaire 3 dépasse la quantité.
    /// assert_eq!(ex.a_part.len(), 1);
    /// assert_eq!(ex.standards(), 0);
    /// ```
    #[must_use]
    pub fn depuis(quantite: i64, commun: Etat, mut a_part: Vec<(i64, Etat)>) -> Self {
        let quantite = quantite.max(0);
        a_part.sort_by_key(|(id, _)| *id);
        a_part.retain(|(_, e)| *e != commun);
        a_part.truncate(usize::try_from(quantite).unwrap_or(0));
        Self {
            quantite,
            commun,
            a_part,
        }
    }

    /// Nombre d'exemplaires standard.
    #[must_use]
    pub fn standards(&self) -> i64 {
        (self.quantite - self.a_part.len() as i64).max(0)
    }

    /// Tous les exemplaires ont-ils le même état ?
    #[must_use]
    pub fn homogene(&self) -> bool {
        match self.a_part.first() {
            None => true,
            Some((_, premier)) => {
                self.standards() == 0 && self.a_part.iter().all(|(_, e)| e == premier)
            }
        }
    }

    /// Chaque exemplaire, un par un : les standard d'abord, puis ceux à part
    /// dans l'ordre où ils ont été réglés.
    #[must_use]
    pub fn liste(&self) -> Vec<(Exemplaire, Etat)> {
        let mut v: Vec<(Exemplaire, Etat)> = (0..self.standards())
            .map(|_| (Exemplaire::Standard, self.commun.clone()))
            .collect();
        v.extend(
            self.a_part
                .iter()
                .map(|(id, e)| (Exemplaire::APart(*id), e.clone())),
        );
        v
    }

    /// Les exemplaires regroupés par état, avec leur nombre : l'état commun
    /// d'abord s'il reste des standard, puis les autres dans l'ordre où ils
    /// apparaissent.
    ///
    /// C'est la forme qu'emploie l'export : une ligne CSV par état.
    ///
    /// ```
    /// use ygo_app::exemplaires::{Etat, Exemplaires};
    /// let nm = Etat::nouveau("NM", "1st");
    /// let pl = Etat::nouveau("PL", "1st");
    /// let ex = Exemplaires::depuis(5, nm.clone(), vec![(1, pl.clone()), (2, pl.clone())]);
    /// assert_eq!(ex.groupes(), vec![(nm, 3), (pl, 2)]);
    /// ```
    #[must_use]
    pub fn groupes(&self) -> Vec<(Etat, i64)> {
        let mut groupes: Vec<(Etat, i64)> = Vec::new();
        if self.standards() > 0 {
            groupes.push((self.commun.clone(), self.standards()));
        }
        for (_, e) in &self.a_part {
            match groupes.iter_mut().find(|(g, _)| g == e) {
                Some((_, n)) => *n += 1,
                None => groupes.push((e.clone(), 1)),
            }
        }
        groupes
    }

    /// Les exemplaires à part à effacer pour descendre à `nouvelle`
    /// quantité : les standard partent d'abord, puis les derniers réglés.
    ///
    /// ```
    /// use ygo_app::exemplaires::{Etat, Exemplaires};
    /// let ex = Exemplaires::depuis(
    ///     4,
    ///     Etat::nouveau("NM", ""),
    ///     vec![(7, Etat::nouveau("PL", "")), (9, Etat::nouveau("DM", ""))],
    /// );
    /// assert!(ex.a_effacer(2).is_empty(), "les deux standard partent");
    /// assert_eq!(ex.a_effacer(1), vec![9], "puis le dernier réglé");
    /// assert_eq!(ex.a_effacer(0), vec![9, 7]);
    /// ```
    #[must_use]
    pub fn a_effacer(&self, nouvelle: i64) -> Vec<i64> {
        let gardes = usize::try_from(nouvelle.max(0)).unwrap_or(0);
        self.a_part
            .iter()
            .skip(gardes)
            .rev()
            .map(|(id, _)| *id)
            .collect()
    }
}

/// Ce que l'utilisateur demande sur les exemplaires d'une ligne.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Demande {
    /// Donner un état à un exemplaire.
    Modifier(Exemplaire, Etat),
    /// Retirer un exemplaire précis — la quantité baisse d'un.
    Retirer(Exemplaire),
    /// Ajouter un exemplaire standard.
    Ajouter,
    /// Écrire un champ sur **tous** les exemplaires, à part compris.
    Tous(Champ, String),
}

/// Crée la table si elle n'existe pas encore.
///
/// # Errors
///
/// Rend une erreur si la base est en lecture seule ou inaccessible.
pub fn creer_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS exemplaires (
             id         INTEGER PRIMARY KEY,
             card_rowid INTEGER NOT NULL,
             qualite    TEXT NOT NULL DEFAULT '',
             edition    TEXT NOT NULL DEFAULT ''
         );
         CREATE INDEX IF NOT EXISTS exemplaires_par_ligne ON exemplaires (card_rowid);",
    )?;
    Ok(())
}

fn table_existe(conn: &Connection) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'exemplaires'",
        [],
        |l| l.get(0),
    )?;
    Ok(n > 0)
}

/// Les exemplaires à part de **tout** un classeur, par `rowid` de ligne.
///
/// Vide quand la table n'existe pas : c'est le cas de tout classeur où rien
/// n'a jamais été réglé à part.
///
/// # Errors
///
/// Rend une erreur si la base est illisible.
pub fn a_part_du_classeur(conn: &Connection) -> Result<HashMap<i64, Vec<(i64, Etat)>>> {
    let mut tout: HashMap<i64, Vec<(i64, Etat)>> = HashMap::new();
    if !table_existe(conn)? {
        return Ok(tout);
    }
    let mut requete =
        conn.prepare("SELECT id, card_rowid, qualite, edition FROM exemplaires ORDER BY id")?;
    let lignes = requete.query_map([], |l| {
        Ok((
            l.get::<_, i64>(1)?,
            l.get::<_, i64>(0)?,
            Etat::nouveau(
                &l.get::<_, Option<String>>(2)?.unwrap_or_default(),
                &l.get::<_, Option<String>>(3)?.unwrap_or_default(),
            ),
        ))
    })?;
    for ligne in lignes {
        let (rowid, id, etat) = ligne?;
        tout.entry(rowid).or_default().push((id, etat));
    }
    Ok(tout)
}

fn a_part_de(conn: &Connection, rowid: i64) -> Result<Vec<(i64, Etat)>> {
    if !table_existe(conn)? {
        return Ok(Vec::new());
    }
    let mut requete = conn.prepare(
        "SELECT id, qualite, edition FROM exemplaires WHERE card_rowid = ?1 ORDER BY id",
    )?;
    let lignes = requete
        .query_map([rowid], |l| {
            Ok((
                l.get::<_, i64>(0)?,
                Etat::nouveau(
                    &l.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    &l.get::<_, Option<String>>(2)?.unwrap_or_default(),
                ),
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(lignes)
}

/// Les exemplaires d'une ligne — `None` si elle n'existe pas.
///
/// # Errors
///
/// Rend une erreur si la base est illisible.
///
/// Un classeur ancien à qui manquerait `qualite` ou `edition` se lit quand
/// même, ces champs vides : rien n'ajoute les colonnes au démarrage.
pub fn lire(conn: &Connection, rowid: i64) -> Result<Option<Exemplaires>> {
    let colonnes = ygo_db::migrations::colonnes(conn, "cards")?;
    let colonne = |nom: &'static str| {
        if colonnes.iter().any(|c| c == nom) {
            nom
        } else {
            "NULL"
        }
    };
    // Les noms viennent d'ici, jamais d'une donnée.
    let requete = format!(
        "SELECT COALESCE(quantite, 0), {}, {} FROM cards WHERE rowid = ?1",
        colonne("qualite"),
        colonne("edition")
    );
    let ligne: Option<(i64, Etat)> = conn
        .query_row(&requete, [rowid], |l| {
            Ok((
                l.get::<_, i64>(0)?,
                Etat::nouveau(
                    &l.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    &l.get::<_, Option<String>>(2)?.unwrap_or_default(),
                ),
            ))
        })
        .optional()?;
    let Some((quantite, commun)) = ligne else {
        return Ok(None);
    };
    Ok(Some(Exemplaires::depuis(
        quantite,
        commun,
        a_part_de(conn, rowid)?,
    )))
}

/// Écrit la quantité d'une ligne, et le drapeau qui en découle.
fn ecrire_quantite(conn: &Connection, rowid: i64, quantite: i64) -> Result<usize> {
    let quantite = quantite.max(0);
    Ok(conn.execute(
        "UPDATE cards SET possessed = ?1, quantite = ?2 WHERE rowid = ?3",
        (i64::from(quantite > 0), quantite, rowid),
    )?)
}

fn effacer_ids(conn: &Connection, ids: &[i64]) -> Result<()> {
    if ids.is_empty() || !table_existe(conn)? {
        return Ok(());
    }
    let mut requete = conn.prepare("DELETE FROM exemplaires WHERE id = ?1")?;
    for id in ids {
        requete.execute([id])?;
    }
    Ok(())
}

/// Règle la quantité d'une ligne en tenant les exemplaires à part : quand
/// elle baisse, les standard partent d'abord.
///
/// N'ouvre pas de transaction : l'appelant qui enchaîne plusieurs lignes en
/// a déjà une.
///
/// Rend le nombre de lignes de `cards` modifiées — `0` si le `rowid`
/// n'existe pas.
///
/// # Errors
///
/// Rend une erreur si la base est inaccessible ou en lecture seule.
///
/// Une ligne qui passe de zéro à au moins un exemplaire reçoit ses
/// [`Defauts`] ([`completer`]).
pub fn regler_quantite(
    conn: &Connection,
    rowid: i64,
    quantite: i64,
    defauts: &Defauts,
) -> Result<usize> {
    let quantite = quantite.max(0);
    let avant: Option<i64> = conn
        .query_row(
            "SELECT COALESCE(quantite, 0) FROM cards WHERE rowid = ?1",
            [rowid],
            |l| l.get(0),
        )
        .optional()?;
    // Sans table, rien à part : exactement l'écriture d'avant.
    if table_existe(conn)? {
        if let Some(ex) = lire(conn, rowid)? {
            effacer_ids(conn, &ex.a_effacer(quantite))?;
        }
    }
    let n = ecrire_quantite(conn, rowid, quantite)?;
    if avant.is_some_and(|a| a <= 0) && quantite > 0 {
        completer(conn, rowid, defauts)?;
    }
    Ok(n)
}

/// Efface les exemplaires à part devenus identiques à l'état commun.
///
/// # Errors
///
/// Rend une erreur si la base est inaccessible ou en lecture seule.
pub fn normaliser(conn: &Connection, rowid: i64) -> Result<()> {
    if !table_existe(conn)? {
        return Ok(());
    }
    conn.execute(
        "DELETE FROM exemplaires
          WHERE card_rowid = ?1
            AND qualite = (SELECT COALESCE(TRIM(qualite), '') FROM cards WHERE rowid = ?1)
            AND edition = (SELECT COALESCE(TRIM(edition), '') FROM cards WHERE rowid = ?1)",
        [rowid],
    )?;
    Ok(())
}

/// Écrit un champ sur **tous** les exemplaires d'une ligne : l'état commun
/// et chaque exemplaire à part. Une valeur vide efface le champ.
///
/// C'est ce que font les actions en masse de l'inventaire : « mettre ces
/// cartes en NM » veut dire chaque exemplaire, pas seulement les standard.
/// Ceux à part gardent l'**autre** champ — un `PL Unlimited` passé en `NM`
/// reste `Unlimited`.
///
/// Rend le nombre de lignes de `cards` modifiées.
///
/// # Errors
///
/// Rend une erreur si la base est inaccessible ou en lecture seule.
pub fn ecrire_pour_tous(
    conn: &Connection,
    rowid: i64,
    champ: Champ,
    valeur: &str,
) -> Result<usize> {
    let valeur = valeur.trim();
    // Les requêtes sont écrites en toutes lettres : la colonne ne vient
    // jamais d'une donnée.
    let (sur_ligne, sur_exemplaires) = match champ {
        Champ::Qualite => (
            "UPDATE cards SET qualite = ?1 WHERE rowid = ?2",
            "UPDATE exemplaires SET qualite = ?1 WHERE card_rowid = ?2",
        ),
        Champ::Edition => (
            "UPDATE cards SET edition = ?1 WHERE rowid = ?2",
            "UPDATE exemplaires SET edition = ?1 WHERE card_rowid = ?2",
        ),
    };
    // Vide : NULL en base, comme l'écrivaient les versions précédentes.
    let en_base: Option<&str> = (!valeur.is_empty()).then_some(valeur);
    let n = conn.execute(sur_ligne, rusqlite::params![en_base, rowid])?;
    if table_existe(conn)? {
        conn.execute(sur_exemplaires, rusqlite::params![valeur, rowid])?;
        normaliser(conn, rowid)?;
    }
    Ok(n)
}

/// Remplace les exemplaires d'une ligne d'un coup : un état commun, une
/// quantité, et des groupes d'exemplaires à part.
///
/// C'est l'écriture de l'import : chaque ligne du CSV devient un groupe, et
/// plus rien ne se perd quand deux lignes d'états différents visent le même
/// tirage.
///
/// # Errors
///
/// Rend une erreur si la base est inaccessible ou en lecture seule.
pub fn remplacer(
    conn: &Connection,
    rowid: i64,
    quantite: i64,
    commun: &Etat,
    a_part: &[(Etat, i64)],
) -> Result<usize> {
    let quantite = quantite.max(0);
    let n = conn.execute(
        "UPDATE cards SET possessed = ?1, quantite = ?2, qualite = ?3, edition = ?4
          WHERE rowid = ?5",
        rusqlite::params![
            i64::from(quantite > 0),
            quantite,
            (!commun.qualite.is_empty()).then_some(commun.qualite.as_str()),
            (!commun.edition.is_empty()).then_some(commun.edition.as_str()),
            rowid,
        ],
    )?;
    let a_ecrire: Vec<&Etat> = a_part
        .iter()
        .filter(|(e, _)| e != commun)
        .flat_map(|(e, k)| std::iter::repeat_n(e, usize::try_from((*k).max(0)).unwrap_or(0)))
        .take(usize::try_from(quantite).unwrap_or(0))
        .collect();
    if table_existe(conn)? {
        conn.execute("DELETE FROM exemplaires WHERE card_rowid = ?1", [rowid])?;
    }
    if !a_ecrire.is_empty() {
        creer_table(conn)?;
        let mut requete = conn.prepare(
            "INSERT INTO exemplaires (card_rowid, qualite, edition) VALUES (?1, ?2, ?3)",
        )?;
        for e in a_ecrire {
            requete.execute(rusqlite::params![rowid, e.qualite, e.edition])?;
        }
    }
    Ok(n)
}

/// Efface tous les exemplaires à part d'un classeur — la remise à zéro.
///
/// # Errors
///
/// Rend une erreur si la base est inaccessible ou en lecture seule.
pub fn tout_effacer(conn: &Connection) -> Result<()> {
    if table_existe(conn)? {
        conn.execute("DELETE FROM exemplaires", [])?;
    }
    Ok(())
}

/// Exécute une demande sur une ligne, **dans une transaction**, et rend les
/// exemplaires tels qu'ils sont devenus — `None` si la ligne n'existe pas.
///
/// # Errors
///
/// Rend une erreur si la base est inaccessible, en lecture seule, ou si
/// l'exemplaire désigné n'existe plus.
///
/// `defauts` sert à « Ajouter » sur une carte qui n'avait aucun exemplaire.
pub fn appliquer(
    conn: &mut Connection,
    rowid: i64,
    demande: &Demande,
    defauts: &Defauts,
) -> Result<Option<Exemplaires>> {
    let tx = conn.transaction()?;
    let Some(ex) = lire(&tx, rowid)? else {
        return Ok(None);
    };
    match demande {
        Demande::Ajouter => {
            ecrire_quantite(&tx, rowid, ex.quantite + 1)?;
            if ex.quantite == 0 {
                completer(&tx, rowid, defauts)?;
            }
        }
        Demande::Tous(champ, valeur) => {
            ecrire_pour_tous(&tx, rowid, *champ, valeur)?;
        }
        Demande::Retirer(Exemplaire::Standard) => {
            if ex.standards() == 0 {
                return Err(introuvable(rowid));
            }
            ecrire_quantite(&tx, rowid, ex.quantite - 1)?;
        }
        Demande::Retirer(Exemplaire::APart(id)) => {
            if !ex.a_part.iter().any(|(i, _)| i == id) {
                return Err(introuvable(rowid));
            }
            effacer_ids(&tx, &[*id])?;
            ecrire_quantite(&tx, rowid, ex.quantite - 1)?;
        }
        Demande::Modifier(Exemplaire::Standard, etat) => {
            if ex.standards() == 0 {
                return Err(introuvable(rowid));
            }
            if *etat != ex.commun {
                creer_table(&tx)?;
                tx.execute(
                    "INSERT INTO exemplaires (card_rowid, qualite, edition) VALUES (?1, ?2, ?3)",
                    rusqlite::params![rowid, etat.qualite, etat.edition],
                )?;
            }
        }
        Demande::Modifier(Exemplaire::APart(id), etat) => {
            if !ex.a_part.iter().any(|(i, _)| i == id) {
                return Err(introuvable(rowid));
            }
            if *etat == ex.commun {
                effacer_ids(&tx, &[*id])?;
            } else {
                tx.execute(
                    "UPDATE exemplaires SET qualite = ?1, edition = ?2 WHERE id = ?3",
                    rusqlite::params![etat.qualite, etat.edition, id],
                )?;
            }
        }
    }
    let apres = lire(&tx, rowid)?;
    tx.commit()?;
    Ok(apres)
}

fn introuvable(rowid: i64) -> AppError {
    AppError::Creation(format!(
        "ligne {rowid} : cet exemplaire n'existe plus — l'affichage était en retard"
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    fn nm() -> Etat {
        Etat::nouveau("NM", "1st")
    }
    fn pl() -> Etat {
        Etat::nouveau("PL", "1st")
    }

    /// Un classeur réduit à ce que le module lit, **sans** table
    /// `exemplaires` — l'état de tous les classeurs existants.
    fn classeur(quantite: i64) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE cards (name TEXT, possessed INTEGER, quantite INTEGER,
                                 qualite TEXT, edition TEXT);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO cards VALUES ('Dark Magician', ?1, ?2, 'NM', '1st')",
            (i64::from(quantite > 0), quantite),
        )
        .unwrap();
        conn
    }

    fn quantite(conn: &Connection) -> (i64, i64) {
        conn.query_row(
            "SELECT possessed, quantite FROM cards WHERE rowid = 1",
            [],
            |l| Ok((l.get(0)?, l.get(1)?)),
        )
        .unwrap()
    }

    #[test]
    fn un_classeur_sans_table_se_lit_comme_avant() {
        let conn = classeur(5);
        let ex = lire(&conn, 1).unwrap().unwrap();
        assert_eq!(ex.quantite, 5);
        assert_eq!(ex.standards(), 5);
        assert!(ex.homogene());
        assert_eq!(ex.groupes(), vec![(nm(), 5)]);
        assert!(a_part_du_classeur(&conn).unwrap().is_empty());
        assert_eq!(lire(&conn, 99).unwrap(), None);
    }

    /// Le cas de la demande : cinq exemplaires, deux abîmés.
    #[test]
    fn deux_exemplaires_sur_cinq_reglés_a_part() {
        let mut conn = classeur(5);
        for _ in 0..2 {
            appliquer(
                &mut conn,
                1,
                &Demande::Modifier(Exemplaire::Standard, pl()),
                &Defauts::default(),
            )
            .unwrap();
        }
        let ex = lire(&conn, 1).unwrap().unwrap();
        assert_eq!(ex.quantite, 5, "la quantité ne bouge pas");
        assert_eq!(ex.standards(), 3);
        assert_eq!(ex.groupes(), vec![(nm(), 3), (pl(), 2)]);
        assert!(!ex.homogene());
        assert_eq!(ex.liste().len(), 5);
    }

    #[test]
    fn un_exemplaire_rendu_a_l_etat_commun_redevient_standard() {
        let mut conn = classeur(3);
        let ex = appliquer(
            &mut conn,
            1,
            &Demande::Modifier(Exemplaire::Standard, pl()),
            &Defauts::default(),
        )
        .unwrap()
        .unwrap();
        let id = ex.a_part[0].0;
        let ex = appliquer(
            &mut conn,
            1,
            &Demande::Modifier(Exemplaire::APart(id), nm()),
            &Defauts::default(),
        )
        .unwrap()
        .unwrap();
        assert!(ex.a_part.is_empty());
        assert_eq!(ex.standards(), 3);
    }

    /// Régler un standard sur l'état qu'il a déjà n'écrit rien.
    #[test]
    fn regler_un_standard_sur_l_etat_commun_ne_cree_rien() {
        let mut conn = classeur(2);
        appliquer(
            &mut conn,
            1,
            &Demande::Modifier(Exemplaire::Standard, nm()),
            &Defauts::default(),
        )
        .unwrap();
        assert!(
            !table_existe(&conn).unwrap(),
            "aucune table créée pour rien"
        );
    }

    /// Décision de l'utilisateur : « − » retire un standard d'abord.
    #[test]
    fn baisser_la_quantite_garde_les_exemplaires_a_part() {
        let mut conn = classeur(4);
        appliquer(
            &mut conn,
            1,
            &Demande::Modifier(Exemplaire::Standard, pl()),
            &Defauts::default(),
        )
        .unwrap();
        regler_quantite(&conn, 1, 2, &Defauts::default()).unwrap();
        let ex = lire(&conn, 1).unwrap().unwrap();
        assert_eq!(ex.groupes(), vec![(nm(), 1), (pl(), 1)]);
        regler_quantite(&conn, 1, 1, &Defauts::default()).unwrap();
        let ex = lire(&conn, 1).unwrap().unwrap();
        assert_eq!(
            ex.groupes(),
            vec![(pl(), 1)],
            "le dernier standard est parti"
        );
        regler_quantite(&conn, 1, 0, &Defauts::default()).unwrap();
        assert!(
            a_part_du_classeur(&conn).unwrap().is_empty(),
            "plus rien à part"
        );
        assert_eq!(quantite(&conn), (0, 0));
    }

    #[test]
    fn retirer_un_exemplaire_precis() {
        let mut conn = classeur(3);
        let ex = appliquer(
            &mut conn,
            1,
            &Demande::Modifier(Exemplaire::Standard, pl()),
            &Defauts::default(),
        )
        .unwrap()
        .unwrap();
        let id = ex.a_part[0].0;
        let ex = appliquer(
            &mut conn,
            1,
            &Demande::Retirer(Exemplaire::APart(id)),
            &Defauts::default(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(ex.groupes(), vec![(nm(), 2)], "c'est le PL qui est parti");
        let ex = appliquer(
            &mut conn,
            1,
            &Demande::Retirer(Exemplaire::Standard),
            &Defauts::default(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(ex.quantite, 1);
        // Un exemplaire qui n'existe plus : l'affichage était en retard.
        assert!(appliquer(
            &mut conn,
            1,
            &Demande::Retirer(Exemplaire::APart(id)),
            &Defauts::default()
        )
        .is_err());
    }

    #[test]
    fn retirer_le_dernier_exemplaire_rend_la_carte_non_possedee() {
        let mut conn = classeur(1);
        appliquer(
            &mut conn,
            1,
            &Demande::Retirer(Exemplaire::Standard),
            &Defauts::default(),
        )
        .unwrap();
        assert_eq!(quantite(&conn), (0, 0));
        appliquer(&mut conn, 1, &Demande::Ajouter, &Defauts::default()).unwrap();
        assert_eq!(quantite(&conn), (1, 1));
    }

    /// « Mettre en NM » vise chaque exemplaire, et chacun garde son édition.
    #[test]
    fn ecrire_pour_tous_touche_les_exemplaires_a_part() {
        let mut conn = classeur(3);
        appliquer(
            &mut conn,
            1,
            &Demande::Modifier(Exemplaire::Standard, Etat::nouveau("PL", "unlimited")),
            &Defauts::default(),
        )
        .unwrap();
        appliquer(
            &mut conn,
            1,
            &Demande::Modifier(Exemplaire::Standard, pl()),
            &Defauts::default(),
        )
        .unwrap();
        appliquer(
            &mut conn,
            1,
            &Demande::Tous(Champ::Qualite, "EX".into()),
            &Defauts::default(),
        )
        .unwrap();
        let ex = lire(&conn, 1).unwrap().unwrap();
        // Le « PL 1st » devient « EX 1st » = l'état commun : il redevient
        // standard. Le « PL unlimited » devient « EX unlimited » et reste à part.
        assert_eq!(
            ex.groupes(),
            vec![
                (Etat::nouveau("EX", "1st"), 2),
                (Etat::nouveau("EX", "unlimited"), 1)
            ]
        );
        // Vider un champ l'efface, à part compris.
        ecrire_pour_tous(&conn, 1, Champ::Edition, "").unwrap();
        let ex = lire(&conn, 1).unwrap().unwrap();
        assert!(ex.homogene());
        let edition: Option<String> = conn
            .query_row("SELECT edition FROM cards WHERE rowid = 1", [], |l| {
                l.get(0)
            })
            .unwrap();
        assert_eq!(edition, None, "vide s'écrit NULL, comme avant");
    }

    #[test]
    fn remplacer_ecrit_les_groupes_de_l_import() {
        let conn = classeur(0);
        remplacer(&conn, 1, 5, &nm(), &[(pl(), 2), (nm(), 9)]).unwrap();
        let ex = lire(&conn, 1).unwrap().unwrap();
        assert_eq!(ex.groupes(), vec![(nm(), 3), (pl(), 2)]);
        assert_eq!(quantite(&conn), (1, 5));
        // Un second import remplace, il n'ajoute pas.
        remplacer(&conn, 1, 2, &pl(), &[]).unwrap();
        let ex = lire(&conn, 1).unwrap().unwrap();
        assert_eq!(ex.groupes(), vec![(pl(), 2)]);
    }

    /// Plus de groupes à part que la quantité : on n'en écrit pas davantage.
    #[test]
    fn remplacer_ne_depasse_pas_la_quantite() {
        let conn = classeur(0);
        remplacer(&conn, 1, 1, &nm(), &[(pl(), 4)]).unwrap();
        let ex = lire(&conn, 1).unwrap().unwrap();
        assert_eq!(ex.groupes(), vec![(pl(), 1)]);
        let lignes: i64 = conn
            .query_row("SELECT count(*) FROM exemplaires", [], |l| l.get(0))
            .unwrap();
        assert_eq!(lignes, 1);
    }

    /// Un classeur sans `qualite` ni `edition` : les boutons « + » et « − »
    /// doivent continuer de marcher, et la lecture rendre des champs vides.
    #[test]
    fn un_classeur_ancien_sans_colonnes_d_etat_reste_utilisable() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE cards (name TEXT, possessed INTEGER, quantite INTEGER);
             INSERT INTO cards VALUES ('X', 1, 2);",
        )
        .unwrap();
        regler_quantite(&conn, 1, 3, &Defauts::default()).unwrap();
        assert_eq!(quantite(&conn), (1, 3));
        let ex = lire(&conn, 1).unwrap().unwrap();
        assert_eq!(ex.groupes(), vec![(Etat::default(), 3)]);
    }

    /// La lecture rétablit les invariants que la base ne tiendrait plus.
    #[test]
    fn la_lecture_ne_croit_pas_une_base_incoherente() {
        let conn = classeur(1);
        creer_table(&conn).unwrap();
        conn.execute_batch(
            "INSERT INTO exemplaires (card_rowid, qualite, edition) VALUES (1, 'PL', '1st');
             INSERT INTO exemplaires (card_rowid, qualite, edition) VALUES (1, 'DM', '1st');
             INSERT INTO exemplaires (card_rowid, qualite, edition) VALUES (1, 'NM', '1st');",
        )
        .unwrap();
        let ex = lire(&conn, 1).unwrap().unwrap();
        assert_eq!(
            ex.liste().len(),
            1,
            "une carte d'un exemplaire en montre un"
        );
        assert_eq!(ex.groupes(), vec![(pl(), 1)]);
    }

    /// Une base de cartes réduite : deux tirages d'une seule édition, un
    /// troisième imprimé en 1st puis en Unlimited.
    fn cardinfo() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE set_prints (set_code TEXT, rarity TEXT, edition TEXT);
             INSERT INTO set_prints VALUES ('RA02-EN001', 'Super Rare', '1st');
             INSERT INTO set_prints VALUES ('RA02-EN001', 'Secret Rare', '1st');
             INSERT INTO set_prints VALUES ('LCKC-EN001', 'Ultra Rare', 'unlimited');
             INSERT INTO set_prints VALUES ('LOB-001', 'Ultra Rare', '1st');
             INSERT INTO set_prints VALUES ('LOB-001', 'Ultra Rare', 'unlimited');",
        )
        .unwrap();
        conn
    }

    fn classeur_a(codes: &[&str]) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE cards (set_code TEXT, possessed INTEGER, quantite INTEGER,
                                 qualite TEXT, edition TEXT);",
        )
        .unwrap();
        for c in codes {
            conn.execute("INSERT INTO cards VALUES (?1, 0, 0, NULL, NULL)", [c])
                .unwrap();
        }
        conn
    }

    fn etat_ligne(conn: &Connection, rowid: i64) -> (Option<String>, Option<String>) {
        conn.query_row(
            "SELECT qualite, edition FROM cards WHERE rowid = ?1",
            [rowid],
            |l| Ok((l.get(0)?, l.get(1)?)),
        )
        .unwrap()
    }

    /// L'édition n'est connue que si la base n'en a qu'une pour ce tirage.
    #[test]
    fn l_edition_n_est_deduite_que_si_elle_est_unique() {
        let classeur = classeur_a(&["RA02-EN001", "LCKC-EN001", "LOB-001", "XXXX-EN001"]);
        let connues = editions_connues(&cardinfo(), &classeur).unwrap();
        assert_eq!(connues.get("RA02-EN001").map(String::as_str), Some("1st"));
        assert_eq!(
            connues.get("LCKC-EN001").map(String::as_str),
            Some("unlimited")
        );
        assert_eq!(
            connues.get("LOB-001"),
            None,
            "1st puis Unlimited : on ne devine pas"
        );
        assert_eq!(connues.get("XXXX-EN001"), None, "inconnu de la base");
    }

    /// Décision du 2026-10-01 : une carte qui entre dans la collection reçoit
    /// Mint et son édition connue — au « + » comme à « Ajouter ».
    #[test]
    fn une_carte_qui_entre_recoit_mint_et_son_edition() {
        let mut classeur = classeur_a(&["RA02-EN001", "LOB-001"]);
        let defauts = Defauts {
            qualite: QUALITE_PAR_DEFAUT.to_owned(),
            editions: editions_connues(&cardinfo(), &classeur).unwrap(),
        };
        regler_quantite(&classeur, 1, 1, &defauts).unwrap();
        assert_eq!(
            etat_ligne(&classeur, 1),
            (Some("M".into()), Some("1st".into()))
        );
        appliquer(&mut classeur, 2, &Demande::Ajouter, &defauts).unwrap();
        assert_eq!(
            etat_ligne(&classeur, 2),
            (Some("M".into()), None),
            "LOB : pas d'édition devinée"
        );
    }

    /// Ce qui est renseigné ne bouge pas, et seule l'entrée dans la
    /// collection pose les défauts.
    #[test]
    fn les_defauts_ne_remplacent_rien() {
        let classeur = classeur_a(&["RA02-EN001"]);
        classeur
            .execute(
                "UPDATE cards SET qualite = 'PL', edition = 'unlimited' WHERE rowid = 1",
                [],
            )
            .unwrap();
        let defauts = Defauts {
            qualite: QUALITE_PAR_DEFAUT.to_owned(),
            editions: editions_connues(&cardinfo(), &classeur).unwrap(),
        };
        regler_quantite(&classeur, 1, 2, &defauts).unwrap();
        assert_eq!(
            etat_ligne(&classeur, 1),
            (Some("PL".into()), Some("unlimited".into()))
        );

        // Une carte déjà possédée dont on a effacé l'état : un « + » de plus
        // ne le repose pas — l'effacement était voulu.
        classeur
            .execute("UPDATE cards SET qualite = NULL WHERE rowid = 1", [])
            .unwrap();
        regler_quantite(&classeur, 1, 3, &defauts).unwrap();
        assert_eq!(etat_ligne(&classeur, 1).0, None);
    }

    /// La reprise de l'existant : compter, puis écrire — et seulement les
    /// lignes possédées.
    #[test]
    fn la_reprise_compte_puis_remplit_les_possedees() {
        let mut classeur = classeur_a(&["RA02-EN001", "LOB-001", "LCKC-EN001"]);
        classeur
            .execute_batch(
                "UPDATE cards SET possessed = 1, quantite = 1 WHERE rowid IN (1, 2);
                 UPDATE cards SET qualite = 'NM' WHERE rowid = 2;",
            )
            .unwrap();
        let defauts = Defauts {
            qualite: QUALITE_PAR_DEFAUT.to_owned(),
            editions: editions_connues(&cardinfo(), &classeur).unwrap(),
        };
        let analyse = completer_classeur(&mut classeur, &defauts, false).unwrap();
        assert_eq!(
            analyse,
            Completion {
                possedees: 2,
                qualites: 1,
                editions: 1,
                editions_inconnues: 1,
            }
        );
        assert_eq!(
            etat_ligne(&classeur, 1),
            (None, None),
            "l'analyse n'écrit rien"
        );
        completer_classeur(&mut classeur, &defauts, true).unwrap();
        assert_eq!(
            etat_ligne(&classeur, 1),
            (Some("M".into()), Some("1st".into()))
        );
        assert_eq!(etat_ligne(&classeur, 2), (Some("NM".into()), None));
        assert_eq!(
            etat_ligne(&classeur, 3),
            (None, None),
            "non possédée : intouchée"
        );
        // Une seconde passe ne trouve plus rien à poser.
        let encore = completer_classeur(&mut classeur, &defauts, false).unwrap();
        assert_eq!((encore.qualites, encore.editions), (0, 0));
    }

    #[test]
    fn la_remise_a_zero_efface_tout() {
        let mut conn = classeur(2);
        appliquer(
            &mut conn,
            1,
            &Demande::Modifier(Exemplaire::Standard, pl()),
            &Defauts::default(),
        )
        .unwrap();
        tout_effacer(&conn).unwrap();
        assert!(a_part_du_classeur(&conn).unwrap().is_empty());
        tout_effacer(&classeur(0)).unwrap();
    }
}
