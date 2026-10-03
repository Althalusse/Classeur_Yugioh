// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Les lignes que la passe artworks a insérées en double.
//!
//! # Ce qu'on répare, et pourquoi ce n'est pas un nettoyage cosmétique
//!
//! Jusqu'à [`crate::artworks::planifier`] corrigé, la passe comparait la
//! rareté du wiki à celle du classeur sans la canoniser. Sur les sets où
//! Yugipedia écrit en abrégé — `RA02` écrit `PlScR`, le classeur écrit
//! `Platinum Secret Rare` — elle ne retrouvait aucune ligne, concluait que le
//! tirage manquait, et **l'insérait**.
//!
//! Relevé sur les neuf classeurs réels de la V1.0.4 : **751 lignes** en trop.
//!
//! | Classeur | Lignes | En trop | Après |
//! |---|---|---|---|
//! | `RA02` | 1 120 | 567 | **553** |
//! | `LDK2` | 262 | 132 | 130 |
//! | `SDLI` | 72 | 36 | 36 |
//! | `EGO1` | 43 | 8 | 35 |
//! | `EGS1` | 46 | 8 | 38 |
//! | `LOCR-JP`, `RA05`, `SDWD`, `VASM` | — | **0** | inchangés |
//!
//! Les 553 lignes qui restent à `RA02` sont exactement ce qu'une création
//! neuve produit : `ygo-cli creer RA02` en écrivait déjà 553.
//!
//! # La règle, et les trois choses qu'elle refuse de faire
//!
//! Une ligne n'est supprimée que si **les trois** conditions tiennent :
//!
//! 1. son libellé de rareté est **non canonique** — la liste des Options ne le
//!    connaît pas ;
//! 2. sa **quantité est nulle** ;
//! 3. une ligne au libellé **canonique** existe dans le même groupe
//!    `(numéro, cadre, rareté canonisée)` — le témoin.
//!
//! Ce qu'elle refuse, et pourquoi chaque refus a un cas réel derrière lui :
//!
//! - **Sans témoin, on ne touche à rien.** `RA05` porte 80 lignes
//!   `PLatinum Secret Rare` qui ne doublent personne : c'est la seule
//!   orthographe présente pour ces tirages. Les supprimer effacerait des
//!   cartes. Elles seront corrigées par `ygo-cli raretes`, pas par ici.
//! - **Une quantité bloque la suppression.** Aucune ligne à supprimer n'en
//!   porte, sur les neuf classeurs — mais la règle ne repose pas sur cette
//!   chance : une ligne possédée est signalée et laissée en place.
//! - **Le `card_image_id` n'entre pas dans la clé.** Il y était d'abord, et il
//!   laissait 49 lignes de `RA02` derrière — celles à qui la passe avait posé
//!   une illustration Yugipedia. À l'inverse, une clé sans la rareté aurait
//!   emporté quatre lignes de `RA05` qui portent une **vraie** variante
//!   d'illustration, dont deux possédées. C'est le libellé non canonique, et
//!   lui seul, qui désigne l'intrus.
//!
//! # Ce que la suppression coûte
//!
//! Les lignes supprimées portaient parfois une illustration Yugipedia que leur
//! témoin n'a pas. Cette illustration n'est pas perdue : elle vit dans
//! `card_images_externes`, et une passe `artworks` relancée après correction la
//! reposera sur la bonne ligne. L'ordre compte donc — dédoublonner, puis
//! relancer les artworks, jamais l'inverse.

use std::collections::{BTreeMap, HashMap};

use rusqlite::Connection;
use ygo_core::rarity::canon;
use ygo_core::rarity::Priorites;

use crate::error::Result;

/// Une ligne de classeur, réduite à ce que la règle regarde.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ligne {
    /// `rowid` dans `cards`.
    pub rowid: i64,
    /// Numéro de collection.
    pub set_code: String,
    /// Libellé de rareté, tel qu'il est en base.
    pub rarity: String,
    /// 1 si Overframe.
    pub extended_art: i64,
    /// Quantité possédée.
    pub quantite: i64,
}

/// Une ligne à supprimer, et la ligne qui la rend superflue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suppression {
    /// La ligne en trop.
    pub rowid: i64,
    /// Son numéro.
    pub set_code: String,
    /// Son libellé, non canonique.
    pub rarity: String,
    /// Le `rowid` de la ligne canonique qui la double.
    pub temoin: i64,
}

/// Une ligne non canonique qui double un témoin **mais porte une quantité**.
///
/// On ne la supprime pas. Elle est signalée pour que quelqu'un tranche.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bloquee {
    /// La ligne concernée.
    pub rowid: i64,
    /// Son numéro.
    pub set_code: String,
    /// Son libellé.
    pub rarity: String,
    /// Ce qu'elle dit posséder.
    pub quantite: i64,
    /// Le `rowid` du témoin.
    pub temoin: i64,
}

/// Ce qu'une analyse a trouvé, sans avoir rien supprimé.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rapport {
    /// Nombre de lignes examinées.
    pub lues: usize,
    /// Lignes à supprimer.
    pub suppressions: Vec<Suppression>,
    /// Lignes non canoniques qu'une quantité protège.
    pub bloquees: Vec<Bloquee>,
    /// Libellés non canoniques **sans témoin**, laissés intacts, et leur
    /// nombre de lignes. C'est le cas de `RA05`.
    pub sans_temoin: BTreeMap<String, usize>,
}

impl Rapport {
    /// Y a-t-il quelque chose à supprimer ?
    #[must_use]
    pub fn vide(&self) -> bool {
        self.suppressions.is_empty()
    }

    /// Le décompte par libellé supprimé, pour l'affichage.
    #[must_use]
    pub fn par_libelle(&self) -> BTreeMap<String, usize> {
        let mut compte = BTreeMap::new();
        for s in &self.suppressions {
            *compte.entry(s.rarity.clone()).or_insert(0_usize) += 1;
        }
        compte
    }
}

/// Décide, sans rien écrire, quelles lignes sont en trop.
///
/// Fonction **pure**, testable sans SQLite.
#[must_use]
pub fn planifier(lignes: &[Ligne], reference: &Priorites) -> Rapport {
    let mut rapport = Rapport {
        lues: lignes.len(),
        ..Rapport::default()
    };

    // Groupe : numéro + cadre + rareté canonisée.
    let mut groupes: HashMap<(String, i64, String), Vec<&Ligne>> = HashMap::new();
    for l in lignes {
        groupes
            .entry((
                l.set_code.clone(),
                l.extended_art,
                canon::cle(&l.rarity, reference),
            ))
            .or_default()
            .push(l);
    }

    // Ordre stable : les `rowid`, pas l'ordre de hachage.
    let mut cles: Vec<&(String, i64, String)> = groupes.keys().collect();
    cles.sort();

    for cle in cles {
        let Some(membres) = groupes.get(cle) else {
            continue;
        };
        let canonique = |l: &Ligne| reference.brute(&l.rarity).is_some();

        // Le témoin : la ligne canonique du groupe. À plusieurs, la première
        // par `rowid` — mais on ne supprime jamais une ligne canonique, donc
        // le choix ne décide que de ce qui s'affiche dans le rapport.
        let temoin = membres
            .iter()
            .filter(|l| canonique(l))
            .map(|l| l.rowid)
            .min();

        for l in membres.iter().filter(|l| !canonique(l)) {
            match temoin {
                None => {
                    *rapport.sans_temoin.entry(l.rarity.clone()).or_insert(0) += 1;
                }
                Some(temoin) if l.quantite > 0 => rapport.bloquees.push(Bloquee {
                    rowid: l.rowid,
                    set_code: l.set_code.clone(),
                    rarity: l.rarity.clone(),
                    quantite: l.quantite,
                    temoin,
                }),
                Some(temoin) => rapport.suppressions.push(Suppression {
                    rowid: l.rowid,
                    set_code: l.set_code.clone(),
                    rarity: l.rarity.clone(),
                    temoin,
                }),
            }
        }
    }

    rapport.suppressions.sort_by_key(|s| s.rowid);
    rapport.bloquees.sort_by_key(|b| b.rowid);
    rapport
}

/// Lit un classeur et rend le plan de dédoublonnage. N'écrit rien.
pub fn analyser(conn: &Connection, reference: &Priorites) -> Result<Rapport> {
    let mut requete = conn.prepare(
        "SELECT rowid, COALESCE(set_code, ''), COALESCE(rarity, ''), \
                COALESCE(extended_art, 0), COALESCE(quantite, 0) \
         FROM cards ORDER BY rowid",
    )?;
    let lignes: Vec<Ligne> = requete
        .query_map([], |l| {
            Ok(Ligne {
                rowid: l.get(0)?,
                set_code: l.get(1)?,
                rarity: l.get(2)?,
                extended_art: l.get(3)?,
                quantite: l.get(4)?,
            })
        })?
        .collect::<std::result::Result<_, _>>()?;
    Ok(planifier(&lignes, reference))
}

/// Supprime les lignes du plan, **toutes dans une seule transaction**.
///
/// Rend le nombre de lignes supprimées.
pub fn appliquer(conn: &mut Connection, rapport: &Rapport) -> Result<usize> {
    let tx = conn.transaction()?;
    let mut supprimees = 0;
    {
        let mut requete = tx.prepare("DELETE FROM cards WHERE rowid = ?1")?;
        for s in &rapport.suppressions {
            supprimees += requete.execute([s.rowid])?;
        }
    }
    tx.commit()?;
    Ok(supprimees)
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
            ("Starlight Rare", 19),
            ("Ultimate Rare", 11),
            ("Quarter Century Secret Rare", 12),
        ])
    }

    fn ligne(rowid: i64, set_code: &str, rarity: &str, quantite: i64) -> Ligne {
        Ligne {
            rowid,
            set_code: set_code.to_owned(),
            rarity: rarity.to_owned(),
            extended_art: 0,
            quantite,
        }
    }

    /// Le cas de `SDWD` : la ligne au `qty` tombé dans la rareté s'efface
    /// devant la vraie.
    ///
    /// # Comment cette paire est née
    ///
    /// YGOPRODeck rend `set_rarity = "3"` pour `SDWD-EN013` — c'est le nombre
    /// d'exemplaires dans le deck, pas une rareté. La ligne est entrée en base
    /// avec ce libellé ; la passe artworks, voyant le fichier Yugipedia
    /// `…-SDWD-EN-C-1E.png` et ne trouvant aucune ligne `Common` pour ce
    /// numéro, en a **inséré une seconde**. D'où deux cartes à l'écran, dont
    /// une qu'aucun import ne remplira jamais.
    ///
    /// Depuis que la canonisation sait qu'un chiffre nu vaut `Common`, les
    /// deux lignes tombent dans le même groupe : le témoin existe, la ligne
    /// numérique est vide, elle part. La possédée reste.
    #[test]
    fn la_ligne_au_nombre_d_exemplaires_part_devant_la_vraie() {
        let lignes = vec![
            ligne(13, "SDWD-EN013", "3", 0),
            ligne(50, "SDWD-EN013", "Common", 1),
            // Une carte du même deck sans doublon : rien ne doit lui arriver.
            ligne(14, "SDWD-EN014", "Common", 1),
        ];
        let rapport = planifier(&lignes, &options());
        assert_eq!(rapport.suppressions.len(), 1);
        assert_eq!(rapport.suppressions[0].rowid, 13);
        assert!(rapport.bloquees.is_empty());
    }

    /// Une ligne numérique **sans** témoin canonique ne part pas.
    ///
    /// Sa rareté est fausse, mais elle est la seule à porter ce numéro :
    /// la supprimer ferait disparaître la carte du classeur. C'est à la
    /// canonisation de la réparer, pas au dédoublonnage de l'effacer.
    #[test]
    fn une_ligne_numerique_seule_de_son_numero_reste() {
        let lignes = vec![ligne(13, "SDWD-EN013", "3", 0)];
        let rapport = planifier(&lignes, &options());
        assert!(
            rapport.suppressions.is_empty(),
            "{:?}",
            rapport.suppressions
        );
    }

    /// Le cas de `RA02` : une ligne canonique possédée, deux abrégées vides.
    #[test]
    fn les_lignes_abregees_qui_doublent_une_canonique_partent() {
        let lignes = vec![
            ligne(1, "RA02-EN001", "Platinum Secret Rare", 1),
            ligne(556, "RA02-EN001", "PlScR", 0),
            ligne(557, "RA02-EN001", "PlScR", 0),
        ];
        let r = planifier(&lignes, &options());
        assert_eq!(
            r.suppressions.iter().map(|s| s.rowid).collect::<Vec<_>>(),
            [556, 557]
        );
        assert!(r.suppressions.iter().all(|s| s.temoin == 1));
        assert!(r.bloquees.is_empty());
        assert!(r.sans_temoin.is_empty());
    }

    /// Le cas de `RA05` : 80 lignes `PLatinum Secret Rare` **sans** jumelle
    /// canonique. Les supprimer effacerait des cartes. On les laisse, et on le
    /// dit — c'est `ygo-cli raretes` qui les corrigera.
    #[test]
    fn une_ligne_non_canonique_sans_temoin_est_intouchable() {
        let lignes = vec![
            ligne(1, "RA05-EN050", "PLatinum Secret Rare", 0),
            ligne(2, "RA05-EN050", "Ultra Rare", 1),
        ];
        let r = planifier(&lignes, &options());
        assert!(r.vide(), "aucune suppression");
        assert_eq!(r.sans_temoin.get("PLatinum Secret Rare"), Some(&1));
    }

    /// Le garde-fou principal : une quantité bloque la suppression, même quand
    /// tout le reste dit « doublon ».
    #[test]
    fn une_quantite_bloque_la_suppression() {
        let lignes = vec![
            ligne(1, "RA02-EN001", "Ultra Rare", 0),
            ligne(2, "RA02-EN001", "UR", 3),
        ];
        let r = planifier(&lignes, &options());
        assert!(r.vide());
        assert_eq!(r.bloquees.len(), 1);
        assert_eq!(r.bloquees[0].rowid, 2);
        assert_eq!(r.bloquees[0].quantite, 3);
        assert_eq!(r.bloquees[0].temoin, 1);
    }

    /// Deux raretés différentes ne se doublent pas, quelle que soit
    /// l'orthographe.
    #[test]
    fn deux_raretes_distinctes_ne_se_doublent_pas() {
        let lignes = vec![
            ligne(1, "RA02-EN001", "Ultra Rare", 1),
            ligne(2, "RA02-EN001", "ScR", 0),
        ];
        let r = planifier(&lignes, &options());
        assert!(r.vide());
        assert_eq!(r.sans_temoin.get("ScR"), Some(&1));
    }

    /// Le cadre sépare : une ligne Overframe ne double pas une ligne normale.
    #[test]
    fn le_cadre_separe_les_groupes() {
        let lignes = vec![
            Ligne {
                extended_art: 1,
                ..ligne(1, "RA02-EN001", "Ultra Rare", 1)
            },
            ligne(2, "RA02-EN001", "UR", 0),
        ];
        let r = planifier(&lignes, &options());
        assert!(r.vide(), "cadres différents, groupes différents");
        assert_eq!(r.sans_temoin.get("UR"), Some(&1));
    }

    /// Le cas de `RA05-EN110` : deux lignes **canoniques** dans le même
    /// groupe, l'une portant une vraie variante d'illustration et une
    /// quantité. Aucune n'est non canonique : la règle ne s'applique pas.
    #[test]
    fn deux_lignes_canoniques_du_meme_groupe_sont_toutes_deux_gardees() {
        let lignes = vec![
            ligne(602, "RA05-EN110", "Starlight Rare", 0),
            ligne(692, "RA05-EN110", "Starlight Rare", 1),
        ];
        let r = planifier(&lignes, &options());
        assert!(r.vide(), "une variante n'est pas un doublon");
        assert!(r.bloquees.is_empty());
        assert!(r.sans_temoin.is_empty());
    }

    fn classeur(lignes: &[Ligne]) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE cards (set_code TEXT, rarity TEXT, extended_art INTEGER, quantite INTEGER)",
            (),
        )
        .unwrap();
        for l in lignes {
            conn.execute(
                "INSERT INTO cards (rowid, set_code, rarity, extended_art, quantite) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                (l.rowid, &l.set_code, &l.rarity, l.extended_art, l.quantite),
            )
            .unwrap();
        }
        conn
    }

    #[test]
    fn l_analyse_lit_sans_supprimer() {
        let lignes = vec![
            ligne(1, "RA02-EN001", "Ultra Rare", 1),
            ligne(2, "RA02-EN001", "UR", 0),
        ];
        let conn = classeur(&lignes);
        let r = analyser(&conn, &options()).unwrap();
        assert_eq!(r.suppressions.len(), 1);
        let restantes: i64 = conn
            .query_row("SELECT count(*) FROM cards", [], |l| l.get(0))
            .unwrap();
        assert_eq!(restantes, 2, "l'analyse ne supprime rien");
    }

    #[test]
    fn l_application_supprime_exactement_le_plan() {
        let lignes = vec![
            ligne(1, "RA02-EN001", "Ultra Rare", 1),
            ligne(2, "RA02-EN001", "UR", 0),
            ligne(3, "RA02-EN002", "Secret Rare", 0),
        ];
        let mut conn = classeur(&lignes);
        let r = analyser(&conn, &options()).unwrap();
        assert_eq!(appliquer(&mut conn, &r).unwrap(), 1);

        let restants: Vec<i64> = conn
            .prepare("SELECT rowid FROM cards ORDER BY rowid")
            .unwrap()
            .query_map([], |l| l.get(0))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        assert_eq!(restants, [1, 3]);
    }

    /// Rejouer la passe ne trouve plus rien : elle peut être lancée sans
    /// registre de ce qui a déjà été fait.
    #[test]
    fn la_passe_est_idempotente() {
        let lignes = vec![
            ligne(1, "RA02-EN001", "Ultra Rare", 1),
            ligne(2, "RA02-EN001", "UR", 0),
        ];
        let mut conn = classeur(&lignes);
        let premier = analyser(&conn, &options()).unwrap();
        appliquer(&mut conn, &premier).unwrap();
        assert!(analyser(&conn, &options()).unwrap().vide());
    }

    #[test]
    fn le_decompte_par_libelle() {
        let lignes = vec![
            ligne(1, "RA02-EN001", "Ultra Rare", 1),
            ligne(2, "RA02-EN001", "UR", 0),
            ligne(3, "RA02-EN002", "Ultra Rare", 0),
            ligne(4, "RA02-EN002", "UR", 0),
            ligne(5, "RA02-EN002", "Secret Rare", 0),
            ligne(6, "RA02-EN002", "ScR", 0),
        ];
        let r = planifier(&lignes, &options());
        let compte = r.par_libelle();
        assert_eq!(compte.get("UR"), Some(&2));
        assert_eq!(compte.get("ScR"), Some(&1));
    }
}
