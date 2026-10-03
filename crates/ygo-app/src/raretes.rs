// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Canonisation des libellés de rareté d'un classeur.
//!
//! La règle de résolution vit dans [`ygo_core::rarity::canon`] — pure, sans
//! SQLite ni réseau. Ce module en fait deux usages :
//!
//! - **à l'écriture**, quand un classeur est créé, pour que rien d'inexact
//!   n'entre ([`canoniser_lignes`]) ;
//! - **en reprise**, sur les classeurs déjà en place ([`analyser`] puis
//!   [`appliquer`]).
//!
//! # Un écart délibéré au portage — le deuxième
//!
//! La règle R9 veut qu'on reproduise le Python, bugs compris. Ici on ne le fait
//! pas, sur demande explicite de l'utilisateur, et pour une raison mesurée :
//! les 912 lignes non canoniques des 26 classeurs réels tombent toutes sur
//! `PRIORITE_INCONNUE_TRI = 9999` au tri et `PRIORITE_INCONNUE_FILTRE = 0` au
//! filtre. Reproduire ce comportement, ce serait reproduire un classeur où
//! `UR` ne se range pas avec `Ultra Rare`.
//!
//! L'oracle n'en souffre pas : la canonisation est **hors** du chemin comparé.
//! Les fixtures figent les libellés tels que le Python les a écrits, et
//! [`analyser`] ne touche à rien tant que [`appliquer`] n'est pas appelé.
//!
//! # Ce que la reprise ne fait pas
//!
//! Elle ne supprime aucune ligne. Sur `RA02`, la canonisation rendra
//! identiques 553 lignes en libellés complets et 567 lignes abrégées — le même
//! set écrit deux fois par deux passes de création successives. Le
//! dédoublonnage est une opération distincte, qui doit d'abord vérifier où
//! sont les quantités possédées ; il n'a pas sa place dans un simple
//! `UPDATE ... SET rarity`.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::Connection;
use ygo_core::paths::Paths;
use ygo_core::rarity::canon::{canoniser, Origine};
use ygo_core::rarity::Priorites;

use crate::creation::LigneClasseur;
use crate::error::Result;

/// Une ligne à réécrire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Correction {
    /// `rowid` de la ligne dans `cards`.
    pub rowid: i64,
    /// Le libellé tel qu'il est en base.
    pub avant: String,
    /// La forme canonique retenue.
    pub apres: String,
    /// Par quelle passe elle a été trouvée.
    pub origine: Origine,
}

/// Ce qu'une analyse a trouvé, sans avoir rien écrit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rapport {
    /// Nombre de lignes examinées.
    pub lues: usize,
    /// Lignes déjà canoniques — rien à faire.
    pub deja: usize,
    /// Lignes à réécrire.
    pub corrections: Vec<Correction>,
    /// Libellés qu'aucune table ne reconnaît, et leur nombre de lignes.
    ///
    /// Ils sont **laissés tels quels**. C'est ici que `force-SMW` apparaît :
    /// la liste est faite pour être lue, pas pour être appliquée.
    pub inconnues: BTreeMap<String, usize>,
}

impl Rapport {
    /// Y a-t-il quelque chose à écrire ?
    #[must_use]
    pub fn vide(&self) -> bool {
        self.corrections.is_empty()
    }

    /// Le décompte par forme canonique visée, pour l'affichage.
    #[must_use]
    pub fn par_cible(&self) -> BTreeMap<(String, String), usize> {
        let mut compte = BTreeMap::new();
        for c in &self.corrections {
            *compte
                .entry((c.avant.clone(), c.apres.clone()))
                .or_insert(0_usize) += 1;
        }
        compte
    }
}

/// Décide, sans rien écrire, ce qu'il faut réécrire.
///
/// Fonction **pure** : elle prend les couples `(rowid, libellé)` déjà lus et
/// rend le plan. C'est la règle R1 — la lecture SQLite est le travail
/// d'[`analyser`], et cette fonction-ci se teste en quatre lignes.
#[must_use]
pub fn planifier(lignes: &[(i64, String)], reference: &Priorites) -> Rapport {
    let mut rapport = Rapport {
        lues: lignes.len(),
        ..Rapport::default()
    };
    for (rowid, libelle) in lignes {
        match canoniser(libelle, reference) {
            Some(canon) if canon.inchange(libelle) => rapport.deja += 1,
            Some(canon) => rapport.corrections.push(Correction {
                rowid: *rowid,
                avant: libelle.clone(),
                apres: canon.libelle,
                origine: canon.origine,
            }),
            None => {
                *rapport.inconnues.entry(libelle.clone()).or_insert(0) += 1;
            }
        }
    }
    rapport
}

/// Lit les raretés d'un classeur et rend le plan de canonisation.
///
/// N'écrit rien.
pub fn analyser(conn: &Connection, reference: &Priorites) -> Result<Rapport> {
    let mut requete =
        conn.prepare("SELECT rowid, COALESCE(rarity, '') FROM cards ORDER BY rowid")?;
    let lignes: Vec<(i64, String)> = requete
        .query_map([], |l| Ok((l.get(0)?, l.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    Ok(planifier(&lignes, reference))
}

/// Écrit les corrections d'un rapport, **toutes dans une seule transaction**.
///
/// Rend le nombre de lignes modifiées. Un `rowid` disparu entre l'analyse et
/// l'application ne fait pas échouer la passe : il compte simplement pour zéro.
pub fn appliquer(conn: &mut Connection, rapport: &Rapport) -> Result<usize> {
    let tx = conn.transaction()?;
    let mut modifiees = 0;
    {
        let mut requete = tx.prepare("UPDATE cards SET rarity = ?1 WHERE rowid = ?2")?;
        for c in &rapport.corrections {
            modifiees += requete.execute((&c.apres, c.rowid))?;
        }
    }
    tx.commit()?;
    Ok(modifiees)
}

/// Bilan d'une canonisation faite à la création, avant écriture en base.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BilanCreation {
    /// Lignes dont le libellé a changé.
    pub corrigees: usize,
    /// Libellés non reconnus, laissés tels quels.
    pub inconnues: BTreeMap<String, usize>,
}

/// Canonise les libellés d'un classeur **avant** son écriture.
///
/// C'est le greffon `corriger_raretes` de [`crate::creation::Greffons`] : il
/// s'insère exactement là où le Python appelait `corriger_rows`.
///
/// Le `rarity_code` n'est pas touché. Il vient d'YGOPRODeck, qui l'a livré
/// apparié au libellé ; le réécrire reviendrait à effacer la trace de ce que la
/// source a dit.
pub fn canoniser_lignes(lignes: &mut [LigneClasseur], reference: &Priorites) -> BilanCreation {
    let mut bilan = BilanCreation::default();
    for ligne in lignes.iter_mut() {
        match canoniser(&ligne.rarity, reference) {
            Some(canon) if canon.inchange(&ligne.rarity) => {}
            Some(canon) => {
                ligne.rarity = canon.libelle;
                bilan.corrigees += 1;
            }
            None => {
                *bilan.inconnues.entry(ligne.rarity.clone()).or_insert(0) += 1;
            }
        }
    }
    bilan
}

/// Les libellés qu'aucune table ne reconnaît, et leur nombre de lignes.
///
/// Le pendant de [`BilanCreation::inconnues`], mais calculé sur un lot de
/// lignes **à un instant donné** — c'est-à-dire après [`ecarter_fantomes`],
/// pour que le décompte consigné dans le classeur soit celui de ce qui y est
/// réellement entré.
#[must_use]
pub fn inconnues(lignes: &[LigneClasseur], reference: &Priorites) -> BTreeMap<String, usize> {
    let mut par_libelle = BTreeMap::new();
    for ligne in lignes {
        if canoniser(&ligne.rarity, reference).is_none() {
            *par_libelle.entry(ligne.rarity.clone()).or_insert(0) += 1;
        }
    }
    par_libelle
}

// ─────────────────────────────────────────────────────────────────────────────
// Les fausses raretés
// ─────────────────────────────────────────────────────────────────────────────

/// Une ligne écartée parce que sa « rareté » n'en est pas une.
///
/// Elle est conservée telle qu'elle était au moment du retrait : c'est la
/// pièce à conviction que l'écran du classeur montre à l'utilisateur, et sans
/// elle un retrait silencieux serait indistinguable d'un trou de la source.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Fantome {
    /// Numéro de la carte — `CH01-EN001`.
    pub set_code: String,
    /// Nom anglais, pour que la ligne se reconnaisse d'un coup d'œil.
    pub name: String,
    /// Le libellé refusé — « New », « New artwork »…
    pub rarete: String,
    /// L'illustration partagée avec les lignes légitimes.
    pub image_id: Option<i64>,
    /// Les raretés reconnues portées par cette même illustration.
    pub jumelles: Vec<String>,
}

/// Retire les lignes dont la « rareté » est un marqueur d'artwork.
///
/// # Le défaut, et pourquoi une garde numérique ne suffisait pas
///
/// YGOPRODeck range dans `set_rarity` des mentions qui ne sont pas des
/// raretés : sur `CH01` et `CH02`, `New` et `New artwork`. La garde posée le
/// 1er septembre testait `chars().all(is_ascii_digit)` — elle attrapait les
/// nombres d'exemplaires, jamais un mot. Mesuré sur la base de l'utilisateur :
/// six lignes par classeur passaient au travers, et se rangeaient au bout du
/// tri sur `PRIORITE_INCONNUE_TRI`.
///
/// # La règle, qui est structurelle et non lexicale
///
/// Une ligne est un fantôme quand **les deux** conditions tiennent :
///
/// 1. son libellé n'est reconnu par aucune table de
///    [`ygo_core::rarity::canon`] ;
/// 2. son `card_image_id` est **aussi** porté par au moins une ligne dont le
///    libellé, lui, est reconnu.
///
/// La seconde condition est ce qui distingue un marqueur d'artwork d'une
/// rareté que le référentiel ignore encore. `force-SMW`, sur `RA05-EN136`, est
/// seul sur son illustration : il reste, comme il doit. Les six `New` de
/// `CH01` partagent la leur avec trois raretés véritables : ils partent.
///
/// # Ce que la règle ne fait pas
///
/// Elle ne renumérote pas `sort_order`. Les trous que le retrait y laisse sont
/// sans effet — la colonne ne sert qu'à `ORDER BY`, et la place dans la grille
/// se calcule sur le rang dans la liste, pas sur sa valeur.
///
/// Elle ne s'applique qu'à la **création**. Un classeur déjà en place garde
/// ses lignes : la quantité possédée peut y avoir été saisie, et aucune règle
/// automatique n'a le droit d'effacer ce que l'utilisateur a compté.
pub fn ecarter_fantomes(lignes: &mut Vec<LigneClasseur>, reference: &Priorites) -> Vec<Fantome> {
    // Les illustrations qui portent au moins une rareté véritable, et
    // lesquelles — la liste sert de justification dans le rapport.
    let mut legitimes: BTreeMap<i64, BTreeSet<String>> = BTreeMap::new();
    for ligne in lignes.iter() {
        let (Some(id), Some(canon)) = (ligne.card_image_id, canoniser(&ligne.rarity, reference))
        else {
            continue;
        };
        legitimes.entry(id).or_default().insert(canon.libelle);
    }

    let mut ecartes = Vec::new();
    lignes.retain(|ligne| {
        let Some(id) = ligne.card_image_id else {
            return true;
        };
        if canoniser(&ligne.rarity, reference).is_some() {
            return true;
        }
        let Some(jumelles) = legitimes.get(&id) else {
            // Seule sur son illustration : c'est peut-être une rareté que le
            // référentiel ne connaît pas encore. On ne tranche pas.
            return true;
        };
        ecartes.push(Fantome {
            set_code: ligne.set_code.clone(),
            name: ligne.name.clone(),
            rarete: ligne.rarity.clone(),
            image_id: Some(id),
            jumelles: jumelles.iter().cloned().collect(),
        });
        false
    });
    ecartes
}

// ─────────────────────────────────────────────────────────────────────────────
// Priorités : génération et synchronisation
// ─────────────────────────────────────────────────────────────────────────────

/// Ce que la synchronisation des priorités a fait.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BilanPriorites {
    /// Le fichier n'existait pas (ou était vide) et vient d'être créé.
    pub cree: bool,
    /// Raretés ajoutées à un fichier déjà là.
    pub ajoutees: usize,
    /// Nombre total de raretés après coup.
    pub total: usize,
}

impl BilanPriorites {
    /// Le fichier a-t-il été écrit ?
    #[must_use]
    pub fn ecrit(self) -> bool {
        self.cree || self.ajoutees > 0
    }
}

/// Les raretés distinctes déclarées par `cardinfo.db`.
///
/// Repli sur [`ygo_core::rarity::RARETES_REPLI`] si la base est absente ou
/// illisible — comme `get_all_rarities_from_db()`. Une installation sans base
/// obtient donc quand même les vingt et une raretés courantes, plutôt que rien.
#[must_use]
pub fn raretes_de_la_base(paths: &Paths) -> Vec<String> {
    let repli = || {
        ygo_core::rarity::RARETES_REPLI
            .iter()
            .map(|r| (*r).to_owned())
            .collect()
    };
    let Ok(conn) = ygo_db::connexion::ouvrir_lecture_seule(paths.cardinfo_db()) else {
        return repli();
    };
    let existe = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='set_prints'",
            [],
            |_| Ok(true),
        )
        .unwrap_or(false);
    if !existe {
        return repli();
    }
    let Ok(mut requete) = conn.prepare(
        "SELECT DISTINCT rarity FROM set_prints
          WHERE rarity IS NOT NULL AND rarity != ''
          ORDER BY rarity",
    ) else {
        return repli();
    };
    let Ok(lignes) = requete.query_map([], |r| r.get::<_, String>(0)) else {
        return repli();
    };
    let raretes: Vec<String> = lignes.flatten().collect();
    if raretes.is_empty() {
        repli()
    } else {
        raretes
    }
}

/// Garantit que `bdd/rarity_config.json` existe et couvre la base.
///
/// # Ce que ceci comble
///
/// [`ygo_core::rarity::priorites_par_defaut`] existait, portée et éprouvée —
/// et **personne ne l'appelait**. Sur une installation vierge,
/// `rarity_config.json` restait donc absent, la table de priorités vide, et
/// toutes les raretés tombaient sur `PRIORITE_INCONNUE_TRI` : le tri par
/// rareté ne rangeait rien, et le filtre « N raretés par artwork » n'avait
/// aucun ordre à appliquer. Mesuré le 2026-09-05 sur un dossier neuf :
/// « rarity_config.json (absent) — 0 rareté(s) déclarée(s) ».
///
/// L'installation de l'utilisateur ne le montrait pas, parce que **le Python
/// avait écrit ce fichier**. C'était le dernier fil qui rattachait le portage
/// à la V1.0.4.
///
/// # La règle, celle du Python
///
/// Portage de `ecran_options._rarete_items_courants()`, qui se répare tout
/// seul en deux temps :
///
/// 1. table absente ou vide → on génère les défauts et on écrit ;
/// 2. rareté présente en base mais absente du fichier → on l'ajoute **à la
///    fin**, au rang suivant, et on réécrit.
///
/// Le second point compte autant que le premier : une base mise à jour peut
/// apporter des raretés qui n'existaient pas, et un fichier figé les laisserait
/// sans ordre. Les priorités déjà écrites ne sont **jamais** renumérotées — ce
/// sont des choix de l'utilisateur.
///
/// # Errors
///
/// Rend une erreur si le fichier ne peut pas être écrit.
pub fn synchroniser_priorites(paths: &Paths) -> Result<(Priorites, BilanPriorites)> {
    let existantes = Priorites::charger(paths.rarity_config());
    let raretes = raretes_de_la_base(paths);

    if existantes.is_empty() {
        let defauts = ygo_core::rarity::priorites_par_defaut(&raretes);
        defauts.enregistrer(paths.rarity_config())?;
        let total = defauts.len();
        tracing::info!(
            total,
            "rarity_config.json créé avec les priorités par défaut"
        );
        return Ok((
            defauts,
            BilanPriorites {
                cree: true,
                ajoutees: 0,
                total,
            },
        ));
    }

    let mut table: Vec<(String, i64)> = existantes
        .iter()
        .map(|(nom, p)| (nom.to_owned(), p))
        .collect();
    let connues: std::collections::HashSet<&str> =
        table.iter().map(|(nom, _)| nom.as_str()).collect();
    let mut suivant = table.iter().map(|(_, p)| *p).max().unwrap_or(0) + 1;
    let mut ajouts: Vec<(String, i64)> = Vec::new();
    for rarete in &raretes {
        if !connues.contains(rarete.as_str()) && !ajouts.iter().any(|(n, _)| n == rarete) {
            ajouts.push((rarete.clone(), suivant));
            suivant += 1;
        }
    }

    let ajoutees = ajouts.len();
    if ajoutees == 0 {
        let total = table.len();
        return Ok((
            existantes,
            BilanPriorites {
                cree: false,
                ajoutees: 0,
                total,
            },
        ));
    }
    table.extend(ajouts);
    let completee = Priorites::depuis_paires(table);
    completee.enregistrer(paths.rarity_config())?;
    let total = completee.len();
    tracing::info!(ajoutees, total, "rarity_config.json complété");
    Ok((
        completee,
        BilanPriorites {
            cree: false,
            ajoutees,
            total,
        },
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    fn options() -> Priorites {
        Priorites::depuis_paires([
            ("Common", 1),
            ("Super Rare", 3),
            ("Ultra Rare", 4),
            ("Secret Rare", 5),
            ("Platinum Secret Rare", 7),
            ("Collector's Rare", 9),
            ("Ultimate Rare", 11),
            ("Short Print", 13),
        ])
    }

    fn lignes() -> Vec<(i64, String)> {
        [
            (1, "Ultra Rare"),
            (2, "UR"),
            (3, "PLatinum Secret Rare"),
            (4, "force-SMW"),
            (5, "ScR"),
            (6, "force-SMW"),
        ]
        .into_iter()
        .map(|(r, s)| (r, s.to_owned()))
        .collect()
    }

    #[test]
    fn le_plan_separe_le_deja_fait_du_a_faire_et_de_l_inconnu() {
        let r = planifier(&lignes(), &options());
        assert_eq!(r.lues, 6);
        assert_eq!(r.deja, 1, "seule « Ultra Rare » était déjà canonique");
        assert_eq!(r.corrections.len(), 3);
        assert_eq!(
            r.inconnues.get("force-SMW"),
            Some(&2),
            "compté, pas corrigé"
        );
        assert_eq!(r.inconnues.len(), 1);
        assert!(!r.vide());
    }

    #[test]
    fn chaque_correction_porte_sa_provenance() {
        let r = planifier(&lignes(), &options());
        let par_rowid: BTreeMap<i64, &Correction> =
            r.corrections.iter().map(|c| (c.rowid, c)).collect();
        assert_eq!(par_rowid[&2].apres, "Ultra Rare");
        assert_eq!(par_rowid[&2].origine, Origine::Abreviation);
        assert_eq!(par_rowid[&3].apres, "Platinum Secret Rare");
        assert_eq!(par_rowid[&3].origine, Origine::Orthographe);
        assert_eq!(par_rowid[&5].apres, "Secret Rare");
    }

    /// Un classeur déjà propre ne produit aucune écriture — c'est ce qui rend
    /// la passe de reprise rejouable sans dommage.
    #[test]
    fn un_classeur_deja_canonique_ne_donne_rien_a_ecrire() {
        let propres: Vec<(i64, String)> = [(1, "Ultra Rare"), (2, "Common"), (3, "Short Print")]
            .into_iter()
            .map(|(r, s)| (r, s.to_owned()))
            .collect();
        let r = planifier(&propres, &options());
        assert!(r.vide());
        assert_eq!(r.deja, 3);
        assert!(r.inconnues.is_empty());
    }

    fn classeur() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE cards (rarity TEXT, quantite INTEGER DEFAULT 0)",
            (),
        )
        .unwrap();
        for (_, r) in lignes() {
            conn.execute("INSERT INTO cards (rarity) VALUES (?1)", [&r])
                .unwrap();
        }
        conn
    }

    #[test]
    fn l_analyse_lit_sans_ecrire() {
        let conn = classeur();
        let r = analyser(&conn, &options()).unwrap();
        assert_eq!(r.corrections.len(), 3);
        let inchange: String = conn
            .query_row("SELECT rarity FROM cards WHERE rowid = 2", [], |l| l.get(0))
            .unwrap();
        assert_eq!(inchange, "UR", "l'analyse n'écrit rien");
    }

    #[test]
    fn l_application_reecrit_exactement_les_lignes_du_plan() {
        let mut conn = classeur();
        let r = analyser(&conn, &options()).unwrap();
        assert_eq!(appliquer(&mut conn, &r).unwrap(), 3);

        let apres: Vec<String> = conn
            .prepare("SELECT rarity FROM cards ORDER BY rowid")
            .unwrap()
            .query_map([], |l| l.get(0))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        assert_eq!(
            apres,
            [
                "Ultra Rare",
                "Ultra Rare",
                "Platinum Secret Rare",
                "force-SMW",
                "Secret Rare",
                "force-SMW",
            ]
        );
    }

    /// Rejouer la passe ne doit plus rien trouver : c'est ce qui autorise à la
    /// lancer sur les 26 classeurs sans tenir de registre de ce qui a été fait.
    #[test]
    fn la_passe_est_idempotente() {
        let mut conn = classeur();
        let premier = analyser(&conn, &options()).unwrap();
        appliquer(&mut conn, &premier).unwrap();

        let second = analyser(&conn, &options()).unwrap();
        assert!(second.vide());
        assert_eq!(second.inconnues.get("force-SMW"), Some(&2));
    }

    /// Une ligne disparue entre l'analyse et l'écriture ne fait pas échouer la
    /// passe : elle compte pour zéro.
    #[test]
    fn une_ligne_disparue_ne_fait_pas_echouer_la_passe() {
        let mut conn = classeur();
        let r = analyser(&conn, &options()).unwrap();
        conn.execute("DELETE FROM cards WHERE rowid = 2", ())
            .unwrap();
        assert_eq!(appliquer(&mut conn, &r).unwrap(), 2);
    }

    /// La canonisation à la création réécrit bien le champ, et **ne touche
    /// pas** au `rarity_code` : celui-ci vient d'YGOPRODeck, qui l'a livré
    /// apparié au libellé. L'effacer effacerait la trace de ce que la source a
    /// dit — et c'est précisément cet appariement qui a servi à construire la
    /// table d'abréviations.
    #[test]
    fn la_canonisation_a_la_creation_reecrit_le_libelle_et_pas_le_code() {
        let mut lignes: Vec<LigneClasseur> = ["UR", "force-SMW", "Ultra Rare"]
            .into_iter()
            .map(|r| LigneClasseur {
                rarity: r.to_owned(),
                rarity_code: "UR".to_owned(),
                ..LigneClasseur::default()
            })
            .collect();

        let bilan = canoniser_lignes(&mut lignes, &options());

        assert_eq!(bilan.corrigees, 1, "seule « UR » change");
        assert_eq!(bilan.inconnues.get("force-SMW"), Some(&1));
        assert_eq!(lignes[0].rarity, "Ultra Rare");
        assert_eq!(lignes[1].rarity, "force-SMW", "laissé tel quel");
        assert_eq!(lignes[2].rarity, "Ultra Rare");
        assert!(
            lignes.iter().all(|l| l.rarity_code == "UR"),
            "le code d'YGOPRODeck n'est pas touché"
        );
    }

    #[test]
    fn le_regroupement_par_cible_compte_les_lignes() {
        let mut lignes = lignes();
        lignes.push((7, "UR".to_owned()));
        let r = planifier(&lignes, &options());
        let cibles = r.par_cible();
        assert_eq!(
            cibles.get(&("UR".to_owned(), "Ultra Rare".to_owned())),
            Some(&2)
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // Les fausses raretés
    // ─────────────────────────────────────────────────────────────────────

    fn ligne(set_code: &str, image: Option<i64>, rarete: &str) -> LigneClasseur {
        LigneClasseur {
            set_code: set_code.to_owned(),
            name: format!("Carte {set_code}"),
            card_image_id: image,
            rarity: rarete.to_owned(),
            ..LigneClasseur::default()
        }
    }

    /// La forme exacte de CH01 : une carte, une illustration, trois raretés
    /// véritables et un marqueur d'artwork glissé par YGOPRODeck.
    fn ch01() -> Vec<LigneClasseur> {
        vec![
            ligne("CH01-EN001", Some(4_007), "Ultra Rare"),
            ligne("CH01-EN001", Some(4_007), "Secret Rare"),
            ligne("CH01-EN001", Some(4_007), "Ultimate Rare"),
            ligne("CH01-EN001", Some(4_007), "New artwork"),
        ]
    }

    #[test]
    fn un_marqueur_d_artwork_partageant_l_image_est_ecarte() {
        let mut lignes = ch01();
        let ecartes = ecarter_fantomes(&mut lignes, &options());

        assert_eq!(lignes.len(), 3, "les trois raretés véritables restent");
        assert_eq!(ecartes.len(), 1);
        assert_eq!(ecartes[0].rarete, "New artwork");
        assert_eq!(ecartes[0].set_code, "CH01-EN001");
        assert_eq!(ecartes[0].image_id, Some(4_007));
        assert_eq!(
            ecartes[0].jumelles,
            vec![
                "Secret Rare".to_owned(),
                "Ultimate Rare".to_owned(),
                "Ultra Rare".to_owned(),
            ],
            "le rapport dit avec quoi la ligne partageait son illustration"
        );
    }

    /// La contre-épreuve, et la raison d'être de la seconde condition :
    /// `force-SMW` est seul sur son illustration. On ne sait pas ce que c'est
    /// — donc on n'y touche pas.
    #[test]
    fn un_libelle_inconnu_seul_sur_son_image_est_garde() {
        let mut lignes = vec![
            ligne("RA05-EN136", Some(9_001), "force-SMW"),
            ligne("RA05-EN137", Some(9_002), "Ultra Rare"),
        ];
        let ecartes = ecarter_fantomes(&mut lignes, &options());

        assert!(ecartes.is_empty());
        assert_eq!(lignes.len(), 2);
        assert_eq!(lignes[0].rarity, "force-SMW");
    }

    /// Le voisinage ne suffit pas : c'est bien l'illustration **partagée** qui
    /// tranche, pas la présence de raretés ailleurs dans le classeur.
    #[test]
    fn le_partage_se_juge_sur_l_image_pas_sur_le_classeur() {
        let mut lignes = vec![
            ligne("XX01-EN001", Some(1), "Ultra Rare"),
            ligne("XX01-EN002", Some(2), "Rareté martienne"),
        ];
        let ecartes = ecarter_fantomes(&mut lignes, &options());
        assert!(
            ecartes.is_empty(),
            "images différentes, aucun rapprochement"
        );
        assert_eq!(lignes.len(), 2);
    }

    /// Sans `card_image_id`, la règle n'a pas de prise — et elle s'abstient.
    #[test]
    fn une_ligne_sans_image_n_est_jamais_ecartee() {
        let mut lignes = vec![
            ligne("XX01-EN001", None, "New"),
            ligne("XX01-EN001", None, "Ultra Rare"),
        ];
        let ecartes = ecarter_fantomes(&mut lignes, &options());
        assert!(ecartes.is_empty());
        assert_eq!(lignes.len(), 2);
    }

    /// Le défaut corrigé le 2026-09-05, dans son autre moitié : « Grand Master
    /// Rare » **est** son propre nom canonique. Avant le correctif, il n'était
    /// reconnu par aucune passe, et cette règle aurait emporté onze lignes
    /// légitimes de LOCH-JP / LOCR-JP.
    #[test]
    fn une_rarete_deja_canonique_n_est_pas_prise_pour_un_fantome() {
        let reference = Priorites::depuis_paires([("Common", 1), ("Ultra Rare", 4)]);
        let mut lignes = vec![
            ligne("LOCH-JP001", Some(5_000), "Ultra Rare"),
            ligne("LOCH-JP001", Some(5_000), "Grand Master Rare"),
        ];
        let ecartes = ecarter_fantomes(&mut lignes, &reference);

        assert!(
            ecartes.is_empty(),
            "« Grand Master Rare » est au référentiel, même absente des Options"
        );
        assert_eq!(lignes.len(), 2);
    }

    /// La règle s'applique après la canonisation : ce sont les libellés déjà
    /// ramenés à leur forme qui servent de jumelles.
    #[test]
    fn le_decompte_des_inconnues_suit_le_retrait() {
        let mut lignes = ch01();
        lignes.push(ligne("CH01-EN002", Some(4_008), "force-SMW"));

        let avant = inconnues(&lignes, &options());
        assert_eq!(avant.get("New artwork"), Some(&1));
        assert_eq!(avant.get("force-SMW"), Some(&1));

        ecarter_fantomes(&mut lignes, &options());

        let apres = inconnues(&lignes, &options());
        assert_eq!(apres.get("New artwork"), None, "la ligne n'est plus là");
        assert_eq!(
            apres.get("force-SMW"),
            Some(&1),
            "celle-là est restée, et reste comptée"
        );
    }

    /// Le trou mesuré le 2026-09-05 : une installation vierge n'avait aucune
    /// priorité, et rien ne les lui donnait.
    #[test]
    fn une_installation_vierge_recoit_ses_priorites() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(tmp.path());
        paths.creer_dossiers().unwrap();
        assert!(Priorites::charger(paths.rarity_config()).is_empty());

        let (table, bilan) = synchroniser_priorites(&paths).unwrap();
        assert!(bilan.cree);
        assert!(bilan.ecrit());
        assert!(paths.rarity_config().is_file(), "le fichier est écrit");

        // Sans base, on retombe sur les 21 raretés courantes — l'application
        // reste utilisable avant même la première initialisation.
        assert_eq!(table.len(), ygo_core::rarity::RARETES_REPLI.len());
        assert_eq!(table.brute("Common"), Some(1));
        assert_eq!(table.brute("Ultra Rare"), Some(4));
        assert!(
            table.brute("Common").unwrap() < table.brute("Secret Rare").unwrap(),
            "et l'ordre est celui du Python"
        );
    }

    /// Deux passages ne changent rien : la synchronisation est idempotente.
    #[test]
    fn une_seconde_synchronisation_ne_touche_a_rien() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(tmp.path());
        paths.creer_dossiers().unwrap();

        let (_, premier) = synchroniser_priorites(&paths).unwrap();
        assert!(premier.cree);
        let (_, second) = synchroniser_priorites(&paths).unwrap();
        assert!(!second.cree);
        assert_eq!(second.ajoutees, 0);
        assert!(!second.ecrit());
        assert_eq!(second.total, premier.total);
    }

    /// Une rareté que la base déclare mais que le fichier ignore s'ajoute **à
    /// la fin** — c'est la règle du Python, et elle protège les choix déjà
    /// faits par l'utilisateur.
    #[test]
    fn une_rarete_nouvelle_s_ajoute_sans_renumeroter_les_autres() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(tmp.path());
        paths.creer_dossiers().unwrap();

        // L'utilisateur a rangé Common après Rare : un choix, pas un défaut.
        Priorites::depuis_paires([("Rare", 1), ("Common", 2)])
            .enregistrer(paths.rarity_config())
            .unwrap();

        // Une base qui déclare une rareté de plus.
        let conn = ygo_db::connexion::ouvrir(paths.cardinfo_db()).unwrap();
        conn.execute_batch(
            "CREATE TABLE set_prints (rarity TEXT);
             INSERT INTO set_prints VALUES ('Rare'), ('Common'), ('Starlight Rare');",
        )
        .unwrap();
        drop(conn);

        let (table, bilan) = synchroniser_priorites(&paths).unwrap();
        assert_eq!(bilan.ajoutees, 1);
        assert_eq!(
            table.brute("Rare"),
            Some(1),
            "le choix de l'utilisateur tient"
        );
        assert_eq!(table.brute("Common"), Some(2));
        assert_eq!(
            table.brute("Starlight Rare"),
            Some(3),
            "la nouvelle prend le rang suivant, pas sa place d'ORDRE_DEFAUT"
        );
    }

    /// Les raretés viennent de la base quand elle en a, du repli sinon.
    #[test]
    fn les_raretes_viennent_de_la_base_quand_elle_existe() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(tmp.path());
        paths.creer_dossiers().unwrap();
        assert_eq!(
            raretes_de_la_base(&paths).len(),
            ygo_core::rarity::RARETES_REPLI.len(),
            "sans base, le repli"
        );

        let conn = ygo_db::connexion::ouvrir(paths.cardinfo_db()).unwrap();
        conn.execute_batch(
            "CREATE TABLE set_prints (rarity TEXT);
             INSERT INTO set_prints VALUES ('Common'), ('Common'), ('Ghost Rare'), (''), (NULL);",
        )
        .unwrap();
        drop(conn);

        let raretes = raretes_de_la_base(&paths);
        assert_eq!(
            raretes,
            vec!["Common".to_owned(), "Ghost Rare".to_owned()],
            "distinctes, triées, sans vide ni NULL"
        );
    }
}
