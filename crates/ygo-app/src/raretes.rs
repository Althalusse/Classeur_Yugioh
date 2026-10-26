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
use std::path::Path;

use rusqlite::Connection;
use ygo_core::paths::Paths;
use ygo_core::rarity::canon::{canoniser, cle, Origine};
use ygo_core::rarity::{region, Priorites};

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
    /// Les raretés véritables qui le démentent : celles de la même
    /// illustration ([`Motif::MemeIllustration`]), ou celles que la Set list
    /// donne au numéro ([`Motif::NonAttestee`]).
    pub jumelles: Vec<String>,
    /// Pourquoi la ligne n'est pas une rareté. Absent des écarts consignés
    /// avant le 2026-10-23 : c'était alors toujours la même illustration.
    #[serde(default)]
    pub motif: Motif,
}

/// Ce qui fait d'une ligne une fausse rareté.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Motif {
    /// Son illustration porte déjà une rareté véritable : un marqueur
    /// d'artwork (« New » sur CH01) — [`ecarter_fantomes`].
    #[default]
    MemeIllustration,
    /// Ni la Set list Yugipedia ni la base ne connaissent cette rareté pour
    /// ce numéro (`force-SMW` sur RA05-EN136) — [`non_attestees`].
    NonAttestee,
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
            motif: Motif::MemeIllustration,
        });
        false
    });
    ecartes
}

/// Rejoue la règle des fausses raretés **sur le classeur écrit**.
///
/// # Pourquoi un second passage — 2026-09-30
///
/// [`ecarter_fantomes`] regarde les lignes **avant** l'écriture. À cet
/// instant, un fantôme peut être seul sur son illustration : la règle
/// s'abstient, faute de jumelle reconnue — et c'est ce qu'on lui demande.
/// Puis la passe artworks crée les tirages manquants du même numéro, et la
/// ligne se retrouve entourée de vraies raretés.
///
/// Mesuré sur `CH01` puis `CH02`, recréés les 2026-09-20 et 2026-09-30 : à
/// chaque fois **4 écartées au premier passage, 2 survivantes** — les
/// « New » de `CH01-EN019`/`EN041` et de `CH02-EN028`/`EN041`, écrites aux
/// rowids 32 et 45 quand leurs sœurs portent les rowids 59 à 64, c'est-à-dire
/// le bloc ajouté après coup. Yugipedia annonce **62 tirages** pour ces deux
/// sets ; le classeur en avait 64.
///
/// # Ce que ce passage ne fait pas
///
/// Il ne touche **jamais** une ligne dont la quantité est renseignée. Une
/// fausse rareté qu'on a comptée n'est plus une erreur de source : c'est une
/// carte que l'utilisateur dit posséder, et aucune règle automatique n'a le
/// droit de l'effacer. Ces lignes sont rendues dans le bilan, marquées, pour
/// que l'écran puisse le dire.
///
/// # Errors
///
/// Rend une erreur si le classeur n'est pas ouvrable en écriture.
/// Une ligne du classeur écrit, telle que le second passage la lit.
///
/// Un tuple à six champs se relit mal, et le nom est ce qui fait la différence
/// entre un écart consigné qu'on peut lire et une liste de codes.
struct LigneEnBase {
    rowid: i64,
    set_code: String,
    name: String,
    rarete: String,
    image: Option<i64>,
    quantite: i64,
}

pub fn ecarter_fantomes_en_base(
    chemin_db: &Path,
    reference: &Priorites,
) -> Result<BilanSecondPassage> {
    // Connexion **simple**, sans les pragmas WAL : le classeur vient d'être
    // écrit et refermé, et une connexion WAL y laisserait un `-wal` orphelin.
    // C'est le test de purge des dossiers résiduels qui l'a dit, deux fois.
    let mut conn = Connection::open(chemin_db)?;
    let lignes: Vec<LigneEnBase> = {
        let mut requete = conn.prepare(
            "SELECT rowid, COALESCE(set_code,''), COALESCE(name,''), COALESCE(rarity,''), \
                    card_image_id, COALESCE(quantite, 0) FROM cards",
        )?;
        requete
            .query_map([], |l| {
                Ok(LigneEnBase {
                    rowid: l.get(0)?,
                    set_code: l.get(1)?,
                    name: l.get(2)?,
                    rarete: l.get(3)?,
                    image: l.get(4)?,
                    quantite: l.get(5)?,
                })
            })
            .and_then(Iterator::collect::<rusqlite::Result<Vec<_>>>)?
    };

    // Les illustrations qui portent au moins une rareté véritable.
    let mut legitimes: BTreeMap<i64, BTreeSet<String>> = BTreeMap::new();
    for ligne in &lignes {
        let (Some(id), Some(canon)) = (ligne.image, canoniser(&ligne.rarete, reference)) else {
            continue;
        };
        legitimes.entry(id).or_default().insert(canon.libelle);
    }

    let mut bilan = BilanSecondPassage::default();
    let mut a_supprimer = Vec::new();
    for ligne in &lignes {
        let Some(id) = ligne.image else { continue };
        if canoniser(&ligne.rarete, reference).is_some() {
            continue;
        }
        let Some(jumelles) = legitimes.get(&id) else {
            continue;
        };
        let fantome = Fantome {
            set_code: ligne.set_code.clone(),
            name: ligne.name.clone(),
            rarete: ligne.rarete.clone(),
            image_id: Some(id),
            jumelles: jumelles.iter().cloned().collect(),
            motif: Motif::MemeIllustration,
        };
        if ligne.quantite > 0 {
            bilan.gardees.push(fantome);
        } else {
            a_supprimer.push(ligne.rowid);
            bilan.ecartees.push(fantome);
        }
    }

    if !a_supprimer.is_empty() {
        let transaction = conn.transaction()?;
        {
            let mut suppression = transaction.prepare("DELETE FROM cards WHERE rowid = ?1")?;
            for rowid in &a_supprimer {
                suppression.execute([rowid])?;
            }
        }
        transaction.commit()?;
    }
    Ok(bilan)
}

/// Ce que le second passage a trouvé sur le classeur écrit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BilanSecondPassage {
    /// Lignes retirées de la base.
    pub ecartees: Vec<Fantome>,
    /// Fantômes **gardés** parce qu'ils portent des exemplaires possédés.
    pub gardees: Vec<Fantome>,
}

impl BilanSecondPassage {
    /// Le passage a-t-il trouvé quoi que ce soit ?
    #[must_use]
    pub fn vide(&self) -> bool {
        self.ecartees.is_empty() && self.gardees.is_empty()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Les raretés que personne n'atteste
// ─────────────────────────────────────────────────────────────────────────────

/// Une ligne du classeur, ce qu'il en faut pour juger sa rareté.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LigneAJuger {
    /// `rowid` de la ligne.
    pub rowid: i64,
    /// Numéro de la carte.
    pub set_code: String,
    /// Nom, pour le rapport.
    pub name: String,
    /// Le libellé de rareté.
    pub rarete: String,
    /// Exemplaires possédés.
    pub quantite: i64,
}

/// Les lignes dont la rareté n'est attestée par personne — fonction **pure**.
///
/// # Le cas — 2026-10-23
///
/// `force-SMW` sur `RA05-EN136` : une annotation qu'YGOPRODeck a laissée
/// dans son champ de rareté. La Set list Yugipedia donne à ce numéro deux
/// raretés, Ultra et Starlight ; la base aussi ; le classeur les a, plus
/// celle-là. [`ecarter_fantomes`] la laisse passer : elle n'a pas
/// d'illustration à elle, rien ne la confronte à une rareté véritable.
///
/// # La règle — cinq conditions, toutes
///
/// 1. son libellé n'est reconnu par aucune table du référentiel : une vraie
///    rareté mal écrite (« UR ») a déjà été ramenée à sa forme ;
/// 2. la Set list (`set_list` : numéro → raretés) connaît le numéro, et pas
///    cette rareté ;
/// 3. chaque rareté que la Set list donne au numéro est déjà sur une
///    **autre** ligne : la ligne ne peut être aucune d'elles sous un autre
///    nom ;
/// 4. la base (`base`, même forme) connaît le numéro, et pas cette rareté :
///    deux sources contre une ;
/// 5. personne n'en possède d'exemplaire — sinon elle est rendue dans
///    `gardees`, à vérifier, jamais retirée : aucune règle automatique
///    n'efface ce que l'utilisateur a compté.
///
/// Une vraie rareté que Yugipedia ne liste pas encore (une réimpression
/// récente) n'est pas touchée : son libellé est reconnu (condition 1).
#[must_use]
pub fn non_attestees(
    lignes: &[LigneAJuger],
    set_list: &BTreeMap<String, Vec<String>>,
    base: &BTreeMap<String, Vec<String>>,
    reference: &Priorites,
) -> NonAttestees {
    let numero = |code: &str| code.trim().to_uppercase();
    // Les raretés que le classeur porte, par numéro et par ligne.
    let mut du_classeur: BTreeMap<String, Vec<(i64, String)>> = BTreeMap::new();
    for l in lignes {
        du_classeur
            .entry(numero(&l.set_code))
            .or_default()
            .push((l.rowid, cle(&l.rarete, reference)));
    }
    let mut verdict = NonAttestees::default();
    for l in lignes {
        if l.rarete.trim().is_empty() || canoniser(&l.rarete, reference).is_some() {
            continue;
        }
        let n = numero(&l.set_code);
        let la_cle = cle(&l.rarete, reference);
        let (Some(attestees), Some(en_base)) = (set_list.get(&n), base.get(&n)) else {
            continue;
        };
        let cles_set_list: BTreeSet<String> = attestees.iter().map(|r| cle(r, reference)).collect();
        if cles_set_list.is_empty() || cles_set_list.contains(&la_cle) {
            continue;
        }
        if en_base.is_empty() || en_base.iter().any(|r| cle(r, reference) == la_cle) {
            continue;
        }
        let autres: BTreeSet<&String> = du_classeur
            .get(&n)
            .into_iter()
            .flatten()
            .filter(|(rowid, _)| *rowid != l.rowid)
            .map(|(_, c)| c)
            .collect();
        if !cles_set_list.iter().all(|c| autres.contains(c)) {
            continue;
        }
        let mut jumelles: Vec<String> = Vec::new();
        for r in attestees {
            let libelle =
                canoniser(r, reference).map_or_else(|| r.trim().to_owned(), |c| c.libelle);
            if !jumelles.contains(&libelle) {
                jumelles.push(libelle);
            }
        }
        let fantome = Fantome {
            set_code: l.set_code.clone(),
            name: l.name.clone(),
            rarete: l.rarete.clone(),
            image_id: None,
            jumelles,
            motif: Motif::NonAttestee,
        };
        if l.quantite > 0 {
            verdict.gardees.push(fantome);
        } else {
            verdict.a_retirer.push((l.rowid, fantome));
        }
    }
    verdict
}

/// Ce que [`non_attestees`] conclut.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NonAttestees {
    /// Lignes à retirer : `rowid`, et la pièce à conviction.
    pub a_retirer: Vec<(i64, Fantome)>,
    /// Lignes qui le seraient, gardées parce que possédées.
    pub gardees: Vec<Fantome>,
}

/// Les numéros dont une ligne porte un libellé que le référentiel ignore :
/// les seuls que [`non_attestees`] peut juger. Sans eux, inutile de lire la
/// Set list.
#[must_use]
pub fn numeros_a_juger(lignes: &[LigneAJuger], reference: &Priorites) -> BTreeSet<String> {
    lignes
        .iter()
        .filter(|l| !l.rarete.trim().is_empty() && canoniser(&l.rarete, reference).is_none())
        .map(|l| l.set_code.trim().to_uppercase())
        .collect()
}

/// Les lignes d'un classeur, telles que [`non_attestees`] les juge.
///
/// # Errors
///
/// Si le classeur ne se lit pas.
pub fn lignes_a_juger(conn: &Connection) -> Result<Vec<LigneAJuger>> {
    let mut q = conn.prepare(
        "SELECT rowid, COALESCE(set_code, ''), COALESCE(name, ''), COALESCE(rarity, ''), \
                COALESCE(quantite, 0) FROM cards",
    )?;
    let lignes = q
        .query_map([], |l| {
            Ok(LigneAJuger {
                rowid: l.get(0)?,
                set_code: l.get(1)?,
                name: l.get(2)?,
                rarete: l.get(3)?,
                quantite: l.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(lignes)
}

/// Les raretés que la base connaît à des numéros : numéro → raretés.
///
/// # Errors
///
/// Si `cardinfo.db` ne se lit pas.
pub fn raretes_en_base(
    cardinfo: &Connection,
    numeros: &BTreeSet<String>,
) -> Result<BTreeMap<String, Vec<String>>> {
    let mut q = cardinfo.prepare_cached(
        "SELECT COALESCE(rarity, '') FROM set_prints \
         WHERE set_code = ?1 AND COALESCE(rarity, '') <> ''",
    )?;
    let mut sortie = BTreeMap::new();
    for numero in numeros {
        let raretes = q
            .query_map([numero], |l| l.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if !raretes.is_empty() {
            sortie.insert(numero.clone(), raretes);
        }
    }
    Ok(sortie)
}

/// Retire du classeur les lignes que [`non_attestees`] désigne, et les
/// consigne dans ses écarts — avec celles gardées parce que possédées.
///
/// # Errors
///
/// Si le classeur ne s'ouvre pas en écriture.
pub fn retirer_non_attestees(
    chemin_db: &Path,
    verdict: &NonAttestees,
    reference: &Priorites,
) -> Result<BilanSecondPassage> {
    let bilan = BilanSecondPassage {
        ecartees: verdict.a_retirer.iter().map(|(_, f)| f.clone()).collect(),
        gardees: verdict.gardees.clone(),
    };
    let mut ecarts = crate::classeur::ecarts_du_classeur(chemin_db).unwrap_or_default();
    // Rien à retirer, rien à signaler, et rien de signalé à effacer.
    if bilan.vide() && ecarts.a_verifier.is_empty() {
        return Ok(bilan);
    }
    // Connexion simple, comme le second passage : pas de `-wal` laissé
    // derrière un classeur fermé.
    let mut conn = Connection::open(chemin_db)?;
    if !verdict.a_retirer.is_empty() {
        let transaction = conn.transaction()?;
        {
            let mut suppression = transaction.prepare("DELETE FROM cards WHERE rowid = ?1")?;
            for (rowid, _) in &verdict.a_retirer {
                suppression.execute([rowid])?;
            }
        }
        transaction.commit()?;
    }
    if ecarts.quand.is_empty() {
        ecarts.quand = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    }
    for f in &bilan.ecartees {
        if !ecarts.fantomes.contains(f) {
            ecarts.fantomes.push(f.clone());
        }
    }
    // Les gardées sont l'état du moment : la liste est refaite, pas allongée.
    ecarts.a_verifier = bilan.gardees.clone();
    ecarts.inconnues = analyser(&conn, reference)?.inconnues;
    crate::classeur::enregistrer_ecarts_sur(&conn, &ecarts)?;
    Ok(bilan)
}

// ─────────────────────────────────────────────────────────────────────────────
// Les noms régionaux — 2026-10-26
// ─────────────────────────────────────────────────────────────────────────────

/// Donne à chaque ligne le nom de sa rareté **dans la région de son numéro**
/// ([`ygo_core::rarity::region`]) : `Ghost Rare` devient `Holographic Rare`
/// sur `INFO-JP033`. Rend le nombre de lignes renommées.
///
/// À la création, avant l'écriture, après la canonisation : la base nomme
/// `Ghost Rare` la Holographic des boosters OCG — c'est le nom que le
/// collectionneur japonais ne lit nulle part, ni sur sa carte, ni chez
/// Yugipedia, ni chez Cardmarket.
pub fn nommer_selon_le_numero(lignes: &mut [LigneClasseur]) -> usize {
    let mut renommees = 0;
    for ligne in lignes.iter_mut() {
        if let Some(nom) = region::nom_pour_le_numero(&ligne.rarity, &ligne.set_code) {
            ligne.rarity = nom.to_owned();
            renommees += 1;
        }
    }
    renommees
}

/// Une ligne de classeur, ce qu'il en faut pour lui donner son nom régional.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LigneRegionale {
    /// `rowid` dans `cards`.
    pub rowid: i64,
    /// Numéro de la carte.
    pub set_code: String,
    /// Libellé de rareté, tel qu'il est en base.
    pub rarete: String,
    /// 1 si Overframe.
    pub extended_art: i64,
    /// Exemplaires possédés.
    pub quantite: i64,
}

/// Ce que le renommage régional d'un classeur déjà écrit décide.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanRegional {
    /// `(rowid, avant, après)` : les lignes renommées.
    pub renommees: Vec<(i64, String, String)>,
    /// Lignes retirées : le même tirage, déjà là sous son nom régional.
    pub retirees: Vec<i64>,
}

impl PlanRegional {
    /// Rien à écrire ?
    #[must_use]
    pub fn vide(&self) -> bool {
        self.renommees.is_empty() && self.retirees.is_empty()
    }

    /// Lignes touchées, renommées ou retirées.
    #[must_use]
    pub fn lignes(&self) -> usize {
        self.renommees.len() + self.retirees.len()
    }
}

/// Le renommage régional d'un classeur **déjà écrit** — fonction pure.
///
/// # Le doublon d'un classeur créé avant le 2026-10-26
///
/// Sa passe artworks, ne reconnaissant pas la Holographic de la Set list
/// dans la Ghost de la base, l'a **insérée** à côté : deux lignes pour une
/// carte. Renommer la Ghost en ferait deux Holographic. D'où la règle, pour
/// une ligne au nom d'une autre région :
///
/// - aucune ligne du même numéro et du même cadre ne porte déjà le bon nom :
///   elle est renommée ;
/// - une ligne le porte déjà — la ligne fait double emploi. Non possédée,
///   elle part. Possédée, elle prend la place d'une jumelle non possédée,
///   qui part ; à défaut, elle est renommée et gardée : rien de ce que
///   l'utilisateur a compté n'est effacé.
#[must_use]
pub fn planifier_noms_regionaux(lignes: &[LigneRegionale]) -> PlanRegional {
    let cle = |l: &LigneRegionale, rarete: &str| {
        (
            l.set_code.trim().to_uppercase(),
            l.extended_art,
            rarete.to_owned(),
        )
    };
    let presentes: BTreeSet<(String, i64, String)> =
        lignes.iter().map(|l| cle(l, &l.rarete)).collect();
    // Les jumelles non possédées : celles qui peuvent céder leur place.
    let mut libres: BTreeMap<(String, i64, String), Vec<i64>> = BTreeMap::new();
    for l in lignes.iter().filter(|l| l.quantite <= 0) {
        libres.entry(cle(l, &l.rarete)).or_default().push(l.rowid);
    }
    let mut plan = PlanRegional::default();
    for l in lignes {
        let Some(nom) = region::nom_pour_le_numero(&l.rarete, &l.set_code) else {
            continue;
        };
        let visee = cle(l, nom);
        if presentes.contains(&visee) {
            if l.quantite <= 0 {
                plan.retirees.push(l.rowid);
                continue;
            }
            if let Some(cedee) = libres.get_mut(&visee).and_then(Vec::pop) {
                plan.retirees.push(cedee);
            }
        }
        plan.renommees
            .push((l.rowid, l.rarete.clone(), nom.to_owned()));
    }
    plan.retirees.sort_unstable();
    plan
}

/// Renomme selon la région les lignes d'un classeur déjà écrit, retire les
/// doublons qu'une ancienne passe artworks a laissés
/// ([`planifier_noms_regionaux`]), puis recompte ses libellés inconnus.
///
/// Le recompte efface le bandeau « libellé non reconnu » d'un classeur créé
/// quand `Holographic Rare` n'était pas au référentiel : les écarts consignés
/// suivent ce que le classeur contient **aujourd'hui** (même règle que
/// `creation::consigner_second_passage`).
///
/// # Errors
///
/// Si le classeur ne s'ouvre pas ou ne s'écrit pas.
pub fn harmoniser_noms_regionaux(chemin_db: &Path, reference: &Priorites) -> Result<PlanRegional> {
    // Un dossier sans base n'en reçoit pas une vide : rien à harmoniser.
    if !chemin_db.is_file() {
        return Ok(PlanRegional::default());
    }
    let ecarts = crate::classeur::ecarts_du_classeur(chemin_db);
    // Connexion simple, comme le second passage : pas de `-wal` laissé
    // derrière un classeur fermé.
    let mut conn = Connection::open(chemin_db)?;
    let colonnes = ygo_db::migrations::colonnes(&conn, "cards")?;
    let colonne = |nom: &str| {
        if colonnes.iter().any(|c| c == nom) {
            format!("COALESCE({nom}, 0)")
        } else {
            "0".to_owned()
        }
    };
    let noms: Vec<&str> = region::NOMS_REGIONAUX
        .iter()
        .flat_map(|(ocg, tcg)| [*ocg, *tcg])
        .collect();
    let marques = vec!["?"; noms.len()].join(", ");
    let sql = format!(
        "SELECT rowid, COALESCE(set_code, ''), COALESCE(rarity, ''), {}, {} \
         FROM cards WHERE rarity IN ({marques})",
        colonne("extended_art"),
        colonne("quantite"),
    );
    let lignes: Vec<LigneRegionale> = {
        let mut requete = conn.prepare(&sql)?;
        let lignes = requete
            .query_map(rusqlite::params_from_iter(noms.iter()), |l| {
                Ok(LigneRegionale {
                    rowid: l.get(0)?,
                    set_code: l.get(1)?,
                    rarete: l.get(2)?,
                    extended_art: l.get(3)?,
                    quantite: l.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        lignes
    };
    let plan = planifier_noms_regionaux(&lignes);
    if !plan.vide() {
        let transaction = conn.transaction()?;
        {
            let mut renommer =
                transaction.prepare("UPDATE cards SET rarity = ?1 WHERE rowid = ?2")?;
            for (rowid, _, apres) in &plan.renommees {
                renommer.execute((apres, rowid))?;
            }
            let mut retirer = transaction.prepare("DELETE FROM cards WHERE rowid = ?1")?;
            for rowid in &plan.retirees {
                retirer.execute([rowid])?;
            }
        }
        transaction.commit()?;
    }
    if let Some(mut ecarts) = ecarts {
        let inconnues = analyser(&conn, reference)?.inconnues;
        if inconnues != ecarts.inconnues {
            ecarts.inconnues = inconnues;
            crate::classeur::enregistrer_ecarts_sur(&conn, &ecarts)?;
        }
    }
    Ok(plan)
}

/// Ajoute à une table de priorités le nom régional d'une rareté qu'elle
/// déclare, **à la même priorité** que sa jumelle : `Holographic Rare` se
/// range avec `Ghost Rare`, que l'utilisateur pourra séparer dans les
/// Options. Rend le nombre d'ajouts.
///
/// La base ne déclare jamais `Holographic Rare` — YGOJSON l'appelle `Ghost
/// Rare` partout —, c'est l'application qui l'écrit dans les classeurs OCG :
/// sans cet ajout, la rareté n'apparaîtrait pas dans la liste des Options.
fn completer_noms_regionaux(table: &mut Vec<(String, i64)>) -> usize {
    let mut ajouts: Vec<(String, i64)> = Vec::new();
    for (ocg, tcg) in region::NOMS_REGIONAUX {
        let priorite = |nom: &str| table.iter().find(|(n, _)| n == nom).map(|(_, p)| *p);
        if let (None, Some(p)) = (priorite(ocg), priorite(tcg)) {
            ajouts.push((ocg.to_owned(), p));
        }
    }
    let n = ajouts.len();
    table.extend(ajouts);
    n
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
/// Un troisième, depuis le 2026-10-26 : le nom régional d'une rareté déclarée
/// — `Holographic Rare`, que l'application écrit et que la base ignore —
/// entre **à la priorité de sa jumelle** (cf. `completer_noms_regionaux`).
///
/// # Errors
///
/// Rend une erreur si le fichier ne peut pas être écrit.
pub fn synchroniser_priorites(paths: &Paths) -> Result<(Priorites, BilanPriorites)> {
    let existantes = Priorites::charger(paths.rarity_config());
    let raretes = raretes_de_la_base(paths);

    if existantes.is_empty() {
        let mut table: Vec<(String, i64)> = ygo_core::rarity::priorites_par_defaut(&raretes)
            .iter()
            .map(|(nom, p)| (nom.to_owned(), p))
            .collect();
        completer_noms_regionaux(&mut table);
        let defauts = Priorites::depuis_paires(table);
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
    let mut ajouts: Vec<(String, i64)> = Vec::new();
    {
        let connues: std::collections::HashSet<&str> =
            table.iter().map(|(nom, _)| nom.as_str()).collect();
        let mut suivant = table.iter().map(|(_, p)| *p).max().unwrap_or(0) + 1;
        for rarete in &raretes {
            if !connues.contains(rarete.as_str()) && !ajouts.iter().any(|(n, _)| n == rarete) {
                ajouts.push((rarete.clone(), suivant));
                suivant += 1;
            }
        }
    }
    let mut ajoutees = ajouts.len();
    table.extend(ajouts);
    // Après les raretés de la base : une Ghost Rare qui vient d'entrer
    // donne aussitôt sa priorité à la Holographic.
    ajoutees += completer_noms_regionaux(&mut table);

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
    use std::path::PathBuf;

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
    // Le second passage, sur la base écrite
    // ─────────────────────────────────────────────────────────────────────

    /// Un classeur jetable, peuplé de `(set_code, rareté, image, quantité)`.
    fn classeur_ecrit(lignes: &[(&str, &str, Option<i64>, i64)]) -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let chemin = tmp.path().join("CH02.db");
        let conn = ygo_db::connexion::ouvrir(&chemin).unwrap();
        conn.execute_batch(ygo_db::schema::DDL_CLASSEUR).unwrap();
        for (code, rarete, image, quantite) in lignes {
            conn.execute(
                "INSERT INTO cards (set_code, rarity, card_image_id, quantite, possessed, name)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'Carte')",
                rusqlite::params![code, rarete, image, quantite, i64::from(*quantite > 0)],
            )
            .unwrap();
        }
        drop(conn);
        (tmp, chemin)
    }

    fn raretes_de(chemin: &Path) -> Vec<String> {
        let conn = ygo_db::connexion::ouvrir_lecture_seule(chemin).unwrap();
        let mut r = conn
            .prepare("SELECT rarity FROM cards ORDER BY rowid")
            .unwrap();
        r.query_map([], |l| l.get::<_, String>(0))
            .unwrap()
            .map(rusqlite::Result::unwrap)
            .collect()
    }

    /// La forme exacte de `CH02-EN028` : la « New » est écrite seule, ses
    /// sœurs arrivent après, par la passe artworks. Le second passage la voit.
    #[test]
    fn le_second_passage_rattrape_ce_que_le_premier_ne_pouvait_pas_voir() {
        let (_tmp, chemin) = classeur_ecrit(&[
            ("CH02-EN028", "New", Some(11_911_336), 0),
            ("CH02-EN001", "Ultra Rare", Some(74_364_659), 0),
            // Les trois sœurs, créées après coup par la passe artworks.
            ("CH02-EN028", "Secret Rare", Some(11_911_336), 0),
            ("CH02-EN028", "Starlight Rare", Some(11_911_336), 0),
            ("CH02-EN028", "Ultra Rare", Some(11_911_336), 0),
        ]);

        let bilan = ecarter_fantomes_en_base(&chemin, &options()).unwrap();

        assert_eq!(bilan.ecartees.len(), 1);
        assert_eq!(bilan.ecartees[0].rarete, "New");
        assert_eq!(bilan.ecartees[0].set_code, "CH02-EN028");
        assert_eq!(
            bilan.ecartees[0].jumelles,
            vec![
                "Secret Rare".to_owned(),
                "Starlight Rare".to_owned(),
                "Ultra Rare".to_owned()
            ]
        );
        assert!(bilan.gardees.is_empty());
        assert_eq!(
            raretes_de(&chemin),
            vec!["Ultra Rare", "Secret Rare", "Starlight Rare", "Ultra Rare"],
            "la « New » est partie, rien d'autre n'a bougé"
        );
    }

    /// Une fausse rareté **possédée** n'est pas effacée : elle est signalée.
    ///
    /// C'est la limite que la règle ne franchit pas — une ligne comptée est
    /// une carte que l'utilisateur dit avoir en main.
    #[test]
    fn un_fantome_possede_est_garde_et_signale() {
        let (_tmp, chemin) = classeur_ecrit(&[
            ("CH02-EN028", "New", Some(11_911_336), 2),
            ("CH02-EN028", "Secret Rare", Some(11_911_336), 0),
        ]);

        let bilan = ecarter_fantomes_en_base(&chemin, &options()).unwrap();

        assert!(bilan.ecartees.is_empty());
        assert_eq!(bilan.gardees.len(), 1);
        assert_eq!(bilan.gardees[0].rarete, "New");
        assert_eq!(raretes_de(&chemin).len(), 2, "rien n'a été supprimé");
        assert!(!bilan.vide());
    }

    /// Les deux abstentions du premier passage valent aussi pour le second :
    /// seul sur son image, ou sans image, on ne tranche pas.
    #[test]
    fn le_second_passage_s_abstient_dans_les_memes_cas() {
        let (_tmp, chemin) = classeur_ecrit(&[
            ("RA05-EN136", "force-SMW", Some(69_272_449), 0),
            ("RA05-EN137", "Ultra Rare", Some(1), 0),
            ("RA05-EN138", "Rareté martienne", None, 0),
        ]);

        let bilan = ecarter_fantomes_en_base(&chemin, &options()).unwrap();

        assert!(bilan.vide());
        assert_eq!(raretes_de(&chemin).len(), 3);
    }

    /// Rejouer le passage ne change plus rien — il est idempotent.
    #[test]
    fn un_second_passage_rejoue_ne_trouve_plus_rien() {
        let (_tmp, chemin) = classeur_ecrit(&[
            ("CH02-EN028", "New artwork", Some(7), 0),
            ("CH02-EN028", "Secret Rare", Some(7), 0),
        ]);
        assert_eq!(
            ecarter_fantomes_en_base(&chemin, &options())
                .unwrap()
                .ecartees
                .len(),
            1
        );
        assert!(ecarter_fantomes_en_base(&chemin, &options())
            .unwrap()
            .vide());
        assert_eq!(raretes_de(&chemin), vec!["Secret Rare"]);
    }

    // ─────────────────────────────────────────────────────────────────────
    // Les raretés que personne n'atteste
    // ─────────────────────────────────────────────────────────────────────

    fn a_juger(rowid: i64, set_code: &str, rarete: &str, quantite: i64) -> LigneAJuger {
        LigneAJuger {
            rowid,
            set_code: set_code.to_owned(),
            name: format!("Carte {set_code}"),
            rarete: rarete.to_owned(),
            quantite,
        }
    }

    /// Les Options d'un classeur RA05 : la Starlight y figure.
    fn options_ra05() -> Priorites {
        Priorites::depuis_paires([("Common", 1), ("Ultra Rare", 4), ("Starlight Rare", 12)])
    }

    /// numéro → raretés, la forme commune de la Set list et de la base.
    fn par_numero(paires: &[(&str, &[&str])]) -> BTreeMap<String, Vec<String>> {
        paires
            .iter()
            .map(|(n, r)| ((*n).to_owned(), r.iter().map(|s| (*s).to_owned()).collect()))
            .collect()
    }

    /// Ce que Yugipedia et la base disent de `RA05-EN136`.
    fn deux_raretes() -> BTreeMap<String, Vec<String>> {
        par_numero(&[("RA05-EN136", &["Ultra Rare", "Starlight Rare"])])
    }

    /// La forme exacte de `RA05-EN136` dans le classeur : les deux raretés
    /// véritables, et l'annotation qu'YGOPRODeck a laissée dans le champ.
    fn ra05() -> Vec<LigneAJuger> {
        vec![
            a_juger(661, "RA05-EN136", "force-SMW", 0),
            a_juger(697, "RA05-EN136", "Ultra Rare", 1),
            a_juger(698, "RA05-EN136", "Starlight Rare", 0),
        ]
    }

    #[test]
    fn une_rarete_que_personne_n_atteste_est_retiree() {
        let verdict = non_attestees(&ra05(), &deux_raretes(), &deux_raretes(), &options_ra05());

        assert!(verdict.gardees.is_empty());
        assert_eq!(verdict.a_retirer.len(), 1);
        let (rowid, fantome) = &verdict.a_retirer[0];
        assert_eq!(*rowid, 661);
        assert_eq!(fantome.rarete, "force-SMW");
        assert_eq!(fantome.set_code, "RA05-EN136");
        assert_eq!(fantome.motif, Motif::NonAttestee);
        assert_eq!(fantome.image_id, None);
        assert_eq!(
            fantome.jumelles,
            vec!["Ultra Rare".to_owned(), "Starlight Rare".to_owned()],
            "dans l'ordre de la Set list"
        );
    }

    /// Condition 5 : possédée, elle est signalée, jamais effacée.
    #[test]
    fn une_rarete_non_attestee_mais_possedee_est_gardee() {
        let mut lignes = ra05();
        lignes[0].quantite = 1;
        let verdict = non_attestees(&lignes, &deux_raretes(), &deux_raretes(), &options_ra05());

        assert!(verdict.a_retirer.is_empty());
        assert_eq!(verdict.gardees.len(), 1);
        assert_eq!(verdict.gardees[0].rarete, "force-SMW");
    }

    /// Conditions 2 et 4 : il faut deux sources qui connaissent le numéro, et
    /// qui ignorent toutes deux la rareté. Une seule ne suffit pas.
    #[test]
    fn sans_deux_sources_qui_la_dementent_on_ne_tranche_pas() {
        let o = options_ra05();
        let rien = BTreeMap::new();
        let avec = par_numero(&[("RA05-EN136", &["Ultra Rare", "Starlight Rare", "force-SMW"])]);

        for (set_list, base) in [
            (&rien, &deux_raretes()),
            (&deux_raretes(), &rien),
            (&avec, &deux_raretes()),
            (&deux_raretes(), &avec),
        ] {
            assert_eq!(
                non_attestees(&ra05(), set_list, base, &o),
                NonAttestees::default()
            );
        }
    }

    /// Condition 3 : une rareté de la Set list manque au classeur — la ligne
    /// inconnue pourrait être elle sous un autre nom. On la laisse.
    #[test]
    fn une_rarete_attestee_absente_du_classeur_retient_la_ligne() {
        let lignes = vec![
            a_juger(661, "RA05-EN136", "force-SMW", 0),
            a_juger(697, "RA05-EN136", "Ultra Rare", 0),
        ];
        let verdict = non_attestees(&lignes, &deux_raretes(), &deux_raretes(), &options_ra05());

        assert_eq!(verdict, NonAttestees::default());
    }

    /// Condition 1 : un libellé reconnu — même abrégé, même absent de la Set
    /// list — n'est jamais jugé ici. Une réimpression que Yugipedia n'a pas
    /// encore listée ne part pas.
    #[test]
    fn une_rarete_reconnue_n_est_jamais_jugee() {
        let lignes = vec![
            a_juger(1, "RA05-EN136", "Ultra Rare", 0),
            a_juger(2, "RA05-EN136", "Starlight Rare", 0),
            a_juger(3, "RA05-EN136", "Secret Rare", 0),
            a_juger(4, "RA05-EN136", "UR", 0),
        ];
        let verdict = non_attestees(&lignes, &deux_raretes(), &deux_raretes(), &options_ra05());

        assert_eq!(verdict, NonAttestees::default());
    }

    /// Seuls les numéros qui portent un libellé inconnu sont à juger : sans
    /// eux, la Set list n'a pas à être lue.
    #[test]
    fn seuls_les_numeros_a_libelle_inconnu_sont_a_juger() {
        let mut lignes = ra05();
        lignes.push(a_juger(700, "RA05-EN137", "Ultra Rare", 0));

        assert_eq!(
            numeros_a_juger(&lignes, &options_ra05()),
            BTreeSet::from(["RA05-EN136".to_owned()])
        );
        assert!(numeros_a_juger(&lignes[1..], &options_ra05()).is_empty());
    }

    /// De bout en bout sur un classeur écrit : la ligne part, la pièce à
    /// conviction est consignée, et rejouer ne retire plus rien.
    #[test]
    fn le_retrait_se_consigne_dans_les_ecarts_du_classeur() {
        let (_tmp, chemin) = classeur_ecrit(&[
            ("RA05-EN136", "Ultra Rare", Some(69_272_449), 1),
            ("RA05-EN136", "force-SMW", Some(69_272_449), 0),
            ("RA05-EN136", "Starlight Rare", Some(69_272_449), 0),
        ]);
        let o = options_ra05();
        let juger = || {
            let conn = ygo_db::connexion::ouvrir_lecture_seule(&chemin).unwrap();
            let lignes = lignes_a_juger(&conn).unwrap();
            non_attestees(&lignes, &deux_raretes(), &deux_raretes(), &o)
        };

        let bilan = retirer_non_attestees(&chemin, &juger(), &o).unwrap();
        assert_eq!(bilan.ecartees.len(), 1);
        assert_eq!(raretes_de(&chemin), vec!["Ultra Rare", "Starlight Rare"]);
        let ecarts = crate::classeur::ecarts_du_classeur(&chemin).unwrap();
        assert_eq!(ecarts.fantomes.len(), 1);
        assert_eq!(ecarts.fantomes[0].motif, Motif::NonAttestee);
        assert!(ecarts.a_verifier.is_empty());
        assert!(
            ecarts.inconnues.is_empty(),
            "le libellé inconnu est parti avec sa ligne"
        );

        let rejoue = juger();
        assert_eq!(rejoue, NonAttestees::default());
        assert!(retirer_non_attestees(&chemin, &rejoue, &o).unwrap().vide());
        assert_eq!(
            crate::classeur::ecarts_du_classeur(&chemin)
                .unwrap()
                .fantomes
                .len(),
            1,
            "consignée une fois"
        );
    }

    /// La liste « à vérifier » est l'état du moment : une ligne possédée y
    /// entre, et en sort quand elle n'est plus possédée et qu'elle est partie.
    #[test]
    fn la_liste_a_verifier_suit_l_etat_du_moment() {
        let (_tmp, chemin) = classeur_ecrit(&[
            ("RA05-EN136", "Ultra Rare", Some(69_272_449), 0),
            ("RA05-EN136", "force-SMW", Some(69_272_449), 2),
            ("RA05-EN136", "Starlight Rare", Some(69_272_449), 0),
        ]);
        let o = options_ra05();
        let juger = || {
            let conn = ygo_db::connexion::ouvrir_lecture_seule(&chemin).unwrap();
            let lignes = lignes_a_juger(&conn).unwrap();
            non_attestees(&lignes, &deux_raretes(), &deux_raretes(), &o)
        };

        let bilan = retirer_non_attestees(&chemin, &juger(), &o).unwrap();
        assert!(bilan.ecartees.is_empty());
        assert_eq!(bilan.gardees.len(), 1);
        assert_eq!(raretes_de(&chemin).len(), 3, "rien n'a été supprimé");
        let ecarts = crate::classeur::ecarts_du_classeur(&chemin).unwrap();
        assert_eq!(ecarts.a_verifier.len(), 1);
        assert!(ecarts.fantomes.is_empty());

        // L'utilisateur remet la quantité à zéro : le passage suivant retire.
        let conn = rusqlite::Connection::open(&chemin).unwrap();
        conn.execute("UPDATE cards SET quantite = 0, possessed = 0", [])
            .unwrap();
        drop(conn);
        retirer_non_attestees(&chemin, &juger(), &o).unwrap();
        let ecarts = crate::classeur::ecarts_du_classeur(&chemin).unwrap();
        assert!(ecarts.a_verifier.is_empty());
        assert_eq!(ecarts.fantomes.len(), 1);
        assert_eq!(raretes_de(&chemin), vec!["Ultra Rare", "Starlight Rare"]);
    }

    /// Les écarts consignés avant le 2026-10-23 n'ont pas de motif : ils se
    /// relisent, et c'était alors toujours la même illustration.
    #[test]
    fn un_ecart_consigne_sans_motif_se_relit() {
        let json = r#"{"set_code":"CH01-EN001","name":"Carte","rarete":"New",
                       "image_id":4007,"jumelles":["Ultra Rare"]}"#;
        let fantome: Fantome = serde_json::from_str(json).unwrap();
        assert_eq!(fantome.motif, Motif::MemeIllustration);
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
        // reste utilisable avant même la première initialisation —, plus le
        // nom OCG de la Ghost Rare, qui en fait partie.
        assert_eq!(table.len(), ygo_core::rarity::RARETES_REPLI.len() + 1);
        assert_eq!(
            table.brute("Holographic Rare"),
            table.brute("Ghost Rare"),
            "à la priorité de sa jumelle"
        );
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

    // ─────────────────────────────────────────────────────────────────────
    // Les noms régionaux — 2026-10-26
    // ─────────────────────────────────────────────────────────────────────

    fn ligne_classeur(set_code: &str, rarete: &str) -> LigneClasseur {
        LigneClasseur {
            set_code: set_code.to_owned(),
            rarity: rarete.to_owned(),
            ..LigneClasseur::default()
        }
    }

    /// À la création : la Ghost Rare de la base devient Holographic sur un
    /// numéro japonais, et reste Ghost sur un numéro TCG.
    #[test]
    fn la_creation_donne_a_chaque_ligne_le_nom_de_sa_region() {
        let mut lignes = vec![
            ligne_classeur("INFO-JP033", "Ghost Rare"),
            ligne_classeur("INFO-JP033", "Ultra Rare"),
            ligne_classeur("LODT-EN000", "Ghost Rare"),
            ligne_classeur("CORI-JPS10", "Ghost Rare"),
        ];
        assert_eq!(nommer_selon_le_numero(&mut lignes), 2);
        let raretes: Vec<&str> = lignes.iter().map(|l| l.rarity.as_str()).collect();
        assert_eq!(
            raretes,
            [
                "Holographic Rare",
                "Ultra Rare",
                "Ghost Rare",
                "Holographic Rare"
            ]
        );
        assert_eq!(
            nommer_selon_le_numero(&mut lignes),
            0,
            "rejouable sans effet"
        );
    }

    fn regionale(
        rowid: i64,
        set_code: &str,
        rarete: &str,
        ext: i64,
        quantite: i64,
    ) -> LigneRegionale {
        LigneRegionale {
            rowid,
            set_code: set_code.to_owned(),
            rarete: rarete.to_owned(),
            extended_art: ext,
            quantite,
        }
    }

    /// Un classeur OCG d'avant le correctif, sans doublon : la Ghost est
    /// renommée, la TCG ne bouge pas.
    #[test]
    fn une_ghost_seule_sur_un_numero_ocg_est_renommee() {
        let plan = planifier_noms_regionaux(&[
            regionale(1, "PHNI-JP033", "Ghost Rare", 0, 0),
            regionale(2, "LODT-EN000", "Ghost Rare", 0, 1),
        ]);
        assert_eq!(
            plan.renommees,
            vec![(1, "Ghost Rare".to_owned(), "Holographic Rare".to_owned())]
        );
        assert!(plan.retirees.is_empty());
    }

    /// Le classeur `INFO-JP` créé le 2026-10-26 : la Ghost de la base et la
    /// Holographic insérée par la passe artworks — une carte, deux lignes.
    /// La Ghost, non possédée, part.
    #[test]
    fn le_doublon_laisse_par_l_ancienne_passe_disparait() {
        let plan = planifier_noms_regionaux(&[
            regionale(31, "INFO-JP033", "Ghost Rare", 0, 0),
            regionale(122, "INFO-JP033", "Holographic Rare", 0, 0),
        ]);
        assert!(plan.renommees.is_empty());
        assert_eq!(plan.retirees, vec![31]);
    }

    /// Possédée, la Ghost garde ses exemplaires : elle prend le nom et la
    /// place de la Holographic vide.
    #[test]
    fn la_ligne_possedee_prend_la_place_de_sa_jumelle_vide() {
        let plan = planifier_noms_regionaux(&[
            regionale(31, "INFO-JP033", "Ghost Rare", 0, 2),
            regionale(122, "INFO-JP033", "Holographic Rare", 0, 0),
        ]);
        assert_eq!(
            plan.renommees,
            vec![(31, "Ghost Rare".to_owned(), "Holographic Rare".to_owned())]
        );
        assert_eq!(plan.retirees, vec![122]);
    }

    /// Les deux possédées : rien n'est effacé, l'utilisateur tranchera.
    #[test]
    fn deux_lignes_possedees_restent_toutes_les_deux() {
        let plan = planifier_noms_regionaux(&[
            regionale(31, "INFO-JP033", "Ghost Rare", 0, 1),
            regionale(122, "INFO-JP033", "Holographic Rare", 0, 1),
        ]);
        assert_eq!(plan.renommees.len(), 1);
        assert!(plan.retirees.is_empty());
    }

    /// Une Holographic Overframe n'est pas la jumelle d'une Ghost au cadre
    /// normal : autre version, renommage simple.
    #[test]
    fn le_cadre_distingue_les_jumelles() {
        let plan = planifier_noms_regionaux(&[
            regionale(1, "INFO-JP033", "Ghost Rare", 0, 0),
            regionale(2, "INFO-JP033", "Holographic Rare", 1, 0),
        ]);
        assert_eq!(plan.renommees.len(), 1);
        assert!(plan.retirees.is_empty());
    }

    /// Sur la base écrite : le doublon part, et le bandeau « libellé non
    /// reconnu » consigné à la création disparaît avec lui — c'est ce que
    /// l'utilisateur voyait sur `INFO-JP`.
    #[test]
    fn l_harmonisation_d_un_classeur_ecrit_efface_le_bandeau() {
        let (_tmp, chemin) = classeur_ecrit(&[
            ("INFO-JP033", "Ultra Rare", Some(1), 0),
            ("INFO-JP033", "Ghost Rare", Some(1), 0),
            ("INFO-JP033", "Holographic Rare", Some(1), 0),
        ]);
        let ecarts = crate::classeur::Ecarts {
            quand: "2026-10-26 12:00:00".to_owned(),
            inconnues: BTreeMap::from([("Holographic Rare".to_owned(), 1)]),
            ..crate::classeur::Ecarts::default()
        };
        crate::classeur::enregistrer_ecarts(&chemin, &ecarts).unwrap();
        assert!(crate::classeur::ecarts_du_classeur(&chemin).is_some());

        let plan = harmoniser_noms_regionaux(&chemin, &options()).unwrap();
        assert_eq!(plan.retirees.len(), 1);
        assert_eq!(raretes_de(&chemin), vec!["Ultra Rare", "Holographic Rare"]);
        assert!(
            crate::classeur::ecarts_du_classeur(&chemin).is_none(),
            "plus rien à signaler"
        );

        // Rejouée, elle ne fait plus rien.
        assert!(harmoniser_noms_regionaux(&chemin, &options())
            .unwrap()
            .vide());
    }

    /// La Holographic entre dans les Options à la priorité de la Ghost — la
    /// base, qui ne la déclare jamais, ne l'y aurait pas mise.
    #[test]
    fn la_holographic_entre_dans_les_options_avec_la_ghost() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(tmp.path());
        paths.creer_dossiers().unwrap();
        Priorites::depuis_paires([("Common", 1), ("Ghost Rare", 19), ("Starlight Rare", 21)])
            .enregistrer(paths.rarity_config())
            .unwrap();
        let conn = ygo_db::connexion::ouvrir(paths.cardinfo_db()).unwrap();
        conn.execute_batch(
            "CREATE TABLE set_prints (rarity TEXT);
             INSERT INTO set_prints VALUES ('Common'), ('Ghost Rare'), ('Starlight Rare');",
        )
        .unwrap();
        drop(conn);

        let (table, bilan) = synchroniser_priorites(&paths).unwrap();
        assert_eq!(bilan.ajoutees, 1);
        assert_eq!(table.brute("Holographic Rare"), Some(19));
        assert_eq!(
            table.brute("Starlight Rare"),
            Some(21),
            "rien n'est renuméroté"
        );

        let (_, second) = synchroniser_priorites(&paths).unwrap();
        assert_eq!(second.ajoutees, 0, "une seule fois");
    }
}
