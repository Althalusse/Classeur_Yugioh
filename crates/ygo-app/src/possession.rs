// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! La possession : quantité et qualité d'une carte du classeur.
//!
//! Portage de `carte_posseder/gestion_carte_posseder.py`.
//!
//! # Un seul écrivain, un seul `UPDATE`
//!
//! `possessed` et `quantite` sont écrits **ensemble**, dans la même
//! instruction, et `possessed` est **dérivé** de la quantité. C'est ce qui
//! explique qu'ils ne se contredisent jamais dans les données réelles : sur les
//! 7 136 cartes figées par l'oracle de l'écran classeur, pas une divergence.
//!
//! Le drapeau reste en base parce que tout le reste de l'application le lit ;
//! mais il n'est la source de vérité de rien. La quantité l'est.
//!
//! # Les deux bascules de classeur entier
//!
//! [`tout_posseder`] et [`tout_remettre_a_zero`] écrivent la même colonne que
//! le reste, en une transaction. Elles n'existent pas dans le Python : seuls
//! deux libellés — `btn.all_possessed`, `btn.none_possessed` — traînent dans
//! son `fr.json`, sans une ligne de code derrière. Ce n'est donc pas un
//! portage, et rien n'oblige à en reproduire les choix.
//!
//! **Ce qu'elles ne touchent pas : l'état et l'édition.** Poser `NM` et
//! `1st Edition` sur 567 lignes d'un geste écrirait 567 fois une information
//! que personne n'a vérifiée, et qui ressortirait telle quelle à l'export
//! Scanflip. La possession se coche ; l'état se renseigne, en masse et
//! sciemment, depuis l'inventaire.
//!
//! **Un exemplaire, pas trois.** Le seuil playset existe pour signaler qu'on
//! peut jouer la carte, pas pour décider ce que l'utilisateur possède.

use rusqlite::Connection;
use ygo_core::paths::Paths;

use crate::error::{AppError, Result};

/// Seuil à partir duquel on considère qu'un *playset* est atteint.
///
/// `PLAYSET_SEUIL` du Python. Trois exemplaires : ce qu'il faut pour jouer une
/// carte en trois copies dans un deck.
pub const SEUIL_PLAYSET: i64 = 3;

/// Règle la quantité d'une carte, et le drapeau qui en découle.
///
/// Portage d'`update_quantite_by_rowid`. Une quantité négative est ramenée à
/// zéro : le Python ne s'en protège pas, mais aucune de ses commandes ne peut
/// en produire — l'incrément par bouton, lui, le pourrait si on se trompait de
/// borne, et une quantité négative rendrait `possessed` faux tout en affichant
/// « ×-1 ».
///
/// Rend le nombre de lignes modifiées — `0` si le `rowid` n'existe pas.
pub fn regler_quantite(conn: &Connection, rowid: i64, quantite: i64) -> Result<usize> {
    let quantite = quantite.max(0);
    let possede = i64::from(quantite > 0);
    Ok(conn.execute(
        "UPDATE cards SET possessed = ?1, quantite = ?2 WHERE rowid = ?3",
        (possede, quantite, rowid),
    )?)
}

/// Ajoute `delta` à la quantité d'une carte, sans descendre sous zéro.
///
/// C'est ce que les boutons `+` et `−` de l'écran appellent. La lecture et
/// l'écriture sont faites dans la **même** transaction : deux clics rapides ne
/// peuvent pas lire la même valeur de départ.
///
/// Rend la nouvelle quantité, ou `None` si la carte n'existe pas.
pub fn ajuster(conn: &mut Connection, rowid: i64, delta: i64) -> Result<Option<i64>> {
    let tx = conn.transaction()?;
    let actuelle: Option<i64> = tx
        .query_row(
            "SELECT COALESCE(quantite, 0) FROM cards WHERE rowid = ?1",
            [rowid],
            |l| l.get(0),
        )
        .ok();
    let Some(actuelle) = actuelle else {
        return Ok(None);
    };
    let nouvelle = actuelle.saturating_add(delta).max(0);
    let possede = i64::from(nouvelle > 0);
    tx.execute(
        "UPDATE cards SET possessed = ?1, quantite = ?2 WHERE rowid = ?3",
        (possede, nouvelle, rowid),
    )?;
    tx.commit()?;
    Ok(Some(nouvelle))
}

/// Marque **toutes** les lignes d'un classeur comme possédées, en un
/// exemplaire.
///
/// Les lignes déjà possédées **gardent leur quantité** : ramener un playset
/// de trois à un ne serait pas « tout posséder », ce serait en perdre deux.
/// Seules les lignes à zéro passent à un.
///
/// L'état et l'édition ne sont pas touchés — cf. tête de module.
///
/// Rend le nombre de lignes réellement modifiées.
///
/// # Errors
///
/// Rend une erreur si la base est inaccessible ou en lecture seule.
pub fn tout_posseder(conn: &Connection) -> Result<usize> {
    Ok(conn.execute(
        "UPDATE cards SET possessed = 1, quantite = 1
          WHERE COALESCE(quantite, 0) <= 0",
        (),
    )?)
}

/// Remet **tout** un classeur à zéro : plus rien n'est possédé.
///
/// # Ce qui part, et ce qui reste
///
/// La quantité, le drapeau, l'état et l'édition. Ces deux derniers
/// **décrivent des exemplaires** : les laisser derrière une quantité nulle
/// donnerait un classeur vide dont les cartes se souviennent d'avoir été
/// `NM 1st Edition` — et l'export, qui ne lit que les possédées, ne les
/// montrerait jamais pour autant.
///
/// Les lignes elles-mêmes restent : elles décrivent les tirages du set, pas
/// la collection. Un classeur remis à zéro est un classeur neuf, pas un
/// classeur supprimé.
///
/// Rend le nombre de lignes réellement modifiées.
///
/// # Errors
///
/// Rend une erreur si la base est inaccessible ou en lecture seule.
pub fn tout_remettre_a_zero(conn: &Connection) -> Result<usize> {
    Ok(conn.execute(
        "UPDATE cards
            SET possessed = 0, quantite = 0, qualite = NULL, edition = NULL
          WHERE COALESCE(quantite, 0) <> 0
             OR COALESCE(possessed, 0) <> 0
             OR qualite IS NOT NULL
             OR edition IS NOT NULL",
        (),
    )?)
}

/// Où en est un classeur : combien de lignes, combien de possédées.
///
/// Se lit **avant** une bascule, pour que la confirmation dise ce qu'elle
/// va faire — et pour que le bouton se désactive quand il n'y a rien à
/// faire.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Etat {
    /// Nombre de lignes du classeur.
    pub lignes: usize,
    /// Nombre de lignes possédées.
    pub possedees: usize,
    /// Somme des quantités.
    pub exemplaires: i64,
}

impl Etat {
    /// Reste-t-il des lignes à cocher ?
    #[must_use]
    pub fn incomplet(&self) -> bool {
        self.possedees < self.lignes
    }

    /// Y a-t-il quelque chose à effacer ?
    #[must_use]
    pub fn a_quelque_chose(&self) -> bool {
        self.possedees > 0
    }
}

/// Lit l'état de possession d'un classeur ouvert.
///
/// # Errors
///
/// Rend une erreur si la base est illisible.
pub fn etat(conn: &Connection) -> Result<Etat> {
    let (lignes, possedees, exemplaires): (i64, i64, i64) = conn.query_row(
        "SELECT COUNT(*),
                COALESCE(SUM(COALESCE(quantite, 0) > 0), 0),
                COALESCE(SUM(COALESCE(quantite, 0)), 0)
           FROM cards",
        [],
        |l| Ok((l.get(0)?, l.get(1)?, l.get(2)?)),
    )?;
    Ok(Etat {
        lignes: usize::try_from(lignes).unwrap_or(0),
        possedees: usize::try_from(possedees).unwrap_or(0),
        exemplaires,
    })
}

/// Laquelle des deux bascules de classeur entier.
///
/// Nommée plutôt que passée en booléen : `basculer(paths, "RA05", true)` ne
/// se relit pas, et ces deux actions-là ne se confondent pas impunément.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bascule {
    /// Cocher tout ce qui ne l'est pas, en un exemplaire.
    ToutPosseder,
    /// Tout remettre à zéro — quantité, état, édition.
    ToutRemettreAZero,
}

impl Bascule {
    /// Le libellé du bouton.
    #[must_use]
    pub fn libelle(self) -> &'static str {
        match self {
            Self::ToutPosseder => "Tout possédé",
            Self::ToutRemettreAZero => "Tout remettre à zéro",
        }
    }
}

/// Ouvre le classeur `code` et lui applique la bascule.
///
/// Une transaction pour tout le classeur : un `UPDATE` de 698 lignes
/// interrompu ne doit pas laisser la moitié d'un set cochée.
///
/// Rend le nombre de lignes modifiées.
///
/// # Errors
///
/// Rend une erreur si le classeur n'existe pas ou n'est pas écrivable.
pub fn appliquer_au_classeur(paths: &Paths, code: &str, bascule: Bascule) -> Result<usize> {
    let mut conn = ygo_db::connexion::ouvrir(paths.classeur_db(code))
        .map_err(|e| AppError::Creation(format!("{code} : {e}")))?;
    let transaction = conn.transaction()?;
    let touchees = match bascule {
        Bascule::ToutPosseder => tout_posseder(&transaction)?,
        Bascule::ToutRemettreAZero => tout_remettre_a_zero(&transaction)?,
    };
    transaction.commit()?;
    Ok(touchees)
}

/// Lit l'état de possession d'un classeur, par son code.
///
/// Un classeur illisible rend [`Etat::default`] plutôt qu'une erreur : cet
/// état ne sert qu'à libeller un bouton et à le désactiver, et un `Etat` à
/// zéro les désactive tous les deux — ce qui est exactement ce qu'il faut
/// faire d'un classeur qu'on n'arrive pas à ouvrir.
#[must_use]
pub fn etat_du_classeur(paths: &Paths, code: &str) -> Etat {
    ygo_db::connexion::ouvrir_lecture_seule(paths.classeur_db(code))
        .ok()
        .and_then(|conn| etat(&conn).ok())
        .unwrap_or_default()
}

/// Règle la qualité d'une carte.
///
/// Portage d'`update_qualite_by_rowid`. Une qualité vide est stockée `NULL`,
/// pas chaîne vide — la colonne a `DEFAULT NULL` et le reste du code teste
/// l'absence, pas la vacuité.
pub fn regler_qualite(conn: &Connection, rowid: i64, qualite: Option<&str>) -> Result<usize> {
    let qualite = qualite.filter(|q| !q.is_empty());
    Ok(conn.execute(
        "UPDATE cards SET qualite = ?1 WHERE rowid = ?2",
        (qualite, rowid),
    )?)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    fn classeur() -> Connection {
        let conn = ygo_db::connexion::en_memoire().unwrap();
        ygo_db::schema::creer_tout(&conn, ygo_db::schema::DDL_CLASSEUR).unwrap();
        conn.execute(
            "INSERT INTO cards (rowid, name, set_code, rarity, possessed, quantite) \
             VALUES (1, 'Dark Magician', 'RA05-EN001', 'Ultra Rare', 0, 0)",
            [],
        )
        .unwrap();
        conn
    }

    fn etat(conn: &Connection, rowid: i64) -> (i64, i64) {
        conn.query_row(
            "SELECT COALESCE(possessed,0), COALESCE(quantite,0) FROM cards WHERE rowid = ?1",
            [rowid],
            |l| Ok((l.get(0)?, l.get(1)?)),
        )
        .unwrap()
    }

    /// Un classeur de plusieurs lignes, aux quantités choisies.
    fn classeur_de(quantites: &[i64]) -> Connection {
        let conn = ygo_db::connexion::en_memoire().unwrap();
        ygo_db::schema::creer_tout(&conn, ygo_db::schema::DDL_CLASSEUR).unwrap();
        for (i, q) in quantites.iter().enumerate() {
            let rowid = i64::try_from(i).unwrap() + 1;
            conn.execute(
                "INSERT INTO cards (rowid, name, set_code, rarity, possessed, quantite) \
                 VALUES (?1, 'Carte', ?2, 'Common', ?3, ?4)",
                rusqlite::params![rowid, format!("X-EN{rowid:03}"), i64::from(*q > 0), q],
            )
            .unwrap();
        }
        conn
    }

    /// « Tout possédé » coche ce qui manque **sans écraser** ce qui est là.
    ///
    /// Ramener un playset de trois à un ne serait pas tout posséder : ce
    /// serait en perdre deux.
    #[test]
    fn tout_posseder_coche_sans_ecraser_les_quantites() {
        let conn = classeur_de(&[0, 3, 0, 1]);
        assert_eq!(tout_posseder(&conn).unwrap(), 2, "seules les deux à zéro");

        let e = super::etat(&conn).unwrap();
        assert_eq!(e.lignes, 4);
        assert_eq!(e.possedees, 4, "tout est coché");
        assert_eq!(e.exemplaires, 1 + 3 + 1 + 1, "le playset a survécu");
        assert!(!e.incomplet());

        // Idempotent : rejouer ne touche plus rien.
        assert_eq!(tout_posseder(&conn).unwrap(), 0);
    }

    /// La remise à zéro emporte aussi l'état et l'édition — ils décrivent
    /// des exemplaires qui n'existent plus.
    #[test]
    fn la_remise_a_zero_emporte_l_etat_et_l_edition() {
        let conn = classeur_de(&[2, 0, 1]);
        conn.execute(
            "UPDATE cards SET qualite = 'NM', edition = '1st' WHERE rowid IN (1, 3)",
            [],
        )
        .unwrap();

        assert_eq!(tout_remettre_a_zero(&conn).unwrap(), 2);
        let e = super::etat(&conn).unwrap();
        assert_eq!(e.lignes, 3, "les lignes restent : elles décrivent le set");
        assert_eq!(e.possedees, 0);
        assert_eq!(e.exemplaires, 0);
        assert!(!e.a_quelque_chose());

        let restes: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM cards WHERE qualite IS NOT NULL OR edition IS NOT NULL",
                [],
                |l| l.get(0),
            )
            .unwrap();
        assert_eq!(restes, 0, "aucun souvenir d'un exemplaire disparu");

        // Idempotent, elle aussi.
        assert_eq!(tout_remettre_a_zero(&conn).unwrap(), 0);
    }

    /// L'état se lit avant la bascule : c'est lui qui désactive le bouton
    /// quand il n'y a rien à faire.
    #[test]
    fn l_etat_dit_s_il_reste_quelque_chose_a_faire() {
        let vide = classeur_de(&[0, 0]);
        let e = super::etat(&vide).unwrap();
        assert!(e.incomplet(), "il reste à cocher");
        assert!(!e.a_quelque_chose(), "mais rien à effacer");

        let plein = classeur_de(&[1, 2]);
        let e = super::etat(&plein).unwrap();
        assert!(!e.incomplet());
        assert!(e.a_quelque_chose());
        assert_eq!(e.exemplaires, 3);

        // Un classeur sans une ligne n'est ni l'un ni l'autre.
        let aucune = classeur_de(&[]);
        let e = super::etat(&aucune).unwrap();
        assert_eq!(e, Etat::default());
        assert!(!e.incomplet());
        assert!(!e.a_quelque_chose());
    }

    /// Le chemin complet : une vraie installation, un vrai classeur, les
    /// deux bascules.
    #[test]
    fn les_bascules_passent_par_le_classeur_sur_disque() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(tmp.path());
        std::fs::create_dir_all(paths.dossier_classeur("RA05")).unwrap();
        let conn = Connection::open(paths.classeur_db("RA05")).unwrap();
        conn.execute_batch(ygo_db::schema::DDL_CLASSEUR).unwrap();
        for rowid in 1..=3 {
            conn.execute(
                "INSERT INTO cards (rowid, name, set_code, rarity, possessed, quantite) \
                 VALUES (?1, 'Carte', ?2, 'Common', 0, 0)",
                rusqlite::params![rowid, format!("RA05-EN{rowid:03}")],
            )
            .unwrap();
        }
        drop(conn);

        let depart = etat_du_classeur(&paths, "RA05");
        assert_eq!(depart.lignes, 3);
        assert!(depart.incomplet() && !depart.a_quelque_chose());

        assert_eq!(
            appliquer_au_classeur(&paths, "RA05", Bascule::ToutPosseder).unwrap(),
            3
        );
        let plein = etat_du_classeur(&paths, "RA05");
        assert_eq!((plein.possedees, plein.exemplaires), (3, 3));

        assert_eq!(
            appliquer_au_classeur(&paths, "RA05", Bascule::ToutRemettreAZero).unwrap(),
            3
        );
        assert_eq!(etat_du_classeur(&paths, "RA05"), depart, "retour au départ");

        // Un classeur qui n'existe pas : pas d'état, et pas de panique.
        assert_eq!(etat_du_classeur(&paths, "INCONNU"), Etat::default());
        assert!(appliquer_au_classeur(&paths, "INCONNU", Bascule::ToutPosseder).is_err());
    }

    #[test]
    fn le_drapeau_est_derive_de_la_quantite() {
        let conn = classeur();
        regler_quantite(&conn, 1, 3).unwrap();
        assert_eq!(etat(&conn, 1), (1, 3));

        regler_quantite(&conn, 1, 0).unwrap();
        assert_eq!(
            etat(&conn, 1),
            (0, 0),
            "zéro exemplaire, carte non possédée"
        );
    }

    #[test]
    fn une_quantite_negative_est_ramenee_a_zero() {
        // Le Python ne s'en protège pas ; un bouton « − » mal borné, si.
        let conn = classeur();
        regler_quantite(&conn, 1, -5).unwrap();
        assert_eq!(etat(&conn, 1), (0, 0));
    }

    #[test]
    fn regler_un_rowid_inexistant_ne_touche_rien() {
        let conn = classeur();
        assert_eq!(regler_quantite(&conn, 999, 4).unwrap(), 0);
        assert_eq!(etat(&conn, 1), (0, 0));
    }

    #[test]
    fn ajuster_incremente_et_decremente() {
        let mut conn = classeur();
        assert_eq!(ajuster(&mut conn, 1, 1).unwrap(), Some(1));
        assert_eq!(ajuster(&mut conn, 1, 1).unwrap(), Some(2));
        assert_eq!(ajuster(&mut conn, 1, -1).unwrap(), Some(1));
        assert_eq!(etat(&conn, 1), (1, 1));
    }

    #[test]
    fn ajuster_ne_descend_jamais_sous_zero() {
        let mut conn = classeur();
        assert_eq!(ajuster(&mut conn, 1, -1).unwrap(), Some(0));
        assert_eq!(ajuster(&mut conn, 1, -10).unwrap(), Some(0));
        assert_eq!(etat(&conn, 1), (0, 0), "et le drapeau suit");
    }

    #[test]
    fn ajuster_une_carte_absente_rend_rien() {
        let mut conn = classeur();
        assert_eq!(ajuster(&mut conn, 999, 1).unwrap(), None);
    }

    #[test]
    fn ajuster_relit_la_quantite_avant_de_l_ecrire() {
        // Deux ajustements successifs doivent se cumuler : si la lecture était
        // faite ailleurs et passée en paramètre, deux clics rapides sur « + »
        // partiraient de la même valeur et n'en compteraient qu'un.
        let mut conn = classeur();
        for _ in 0..5 {
            ajuster(&mut conn, 1, 1).unwrap();
        }
        assert_eq!(etat(&conn, 1), (1, 5));
    }

    #[test]
    fn un_grand_delta_ne_deborde_pas() {
        let mut conn = classeur();
        regler_quantite(&conn, 1, i64::MAX).unwrap();
        assert_eq!(ajuster(&mut conn, 1, 1).unwrap(), Some(i64::MAX));
    }

    #[test]
    fn la_qualite_vide_est_stockee_nulle() {
        let conn = classeur();
        regler_qualite(&conn, 1, Some("Near Mint")).unwrap();
        let lue: Option<String> = conn
            .query_row("SELECT qualite FROM cards WHERE rowid = 1", [], |l| {
                l.get(0)
            })
            .unwrap();
        assert_eq!(lue.as_deref(), Some("Near Mint"));

        regler_qualite(&conn, 1, Some("")).unwrap();
        let vide: Option<String> = conn
            .query_row("SELECT qualite FROM cards WHERE rowid = 1", [], |l| {
                l.get(0)
            })
            .unwrap();
        assert_eq!(vide, None, "vide vaut NULL, pas chaîne vide");

        regler_qualite(&conn, 1, None).unwrap();
        let nulle: Option<String> = conn
            .query_row("SELECT qualite FROM cards WHERE rowid = 1", [], |l| {
                l.get(0)
            })
            .unwrap();
        assert_eq!(nulle, None);
    }

    #[test]
    fn le_seuil_de_playset_est_de_trois() {
        assert_eq!(SEUIL_PLAYSET, 3);
    }
}
