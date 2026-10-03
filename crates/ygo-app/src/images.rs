// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! La passe d'images d'un classeur : lire, planifier, télécharger.
//!
//! La décision vit dans [`ygo_images::planifier`], qui est pure. Ce module
//! fournit les deux choses qu'elle ne connaît pas : les lignes du classeur, et
//! la recherche d'URL de repli dans `cardinfo.db`.
//!
//! # Pourquoi la recherche de repli est passée en fermeture
//!
//! `build_fallback_url` du Python interroge `cardinfo.db` pour trouver l'URL
//! Yugipedia correspondant à un identifiant YGOPRODeck. C'est du SQLite, donc
//! cela ne peut pas vivre dans `ygo-core` (règle R1) ni dans `ygo-images`, qui
//! ne connaît que des URL et des octets. La fermeture est construite ici, où
//! les deux bases sont ouvertes.

use std::path::Path;

use rusqlite::Connection;
use ygo_core::config::SourceImage;
use ygo_core::paths::Paths;
use ygo_images::plan::Retour;
use ygo_images::{planifier, Bilan, LigneImage, Telechargeur};

use crate::error::Result;

/// Lit les lignes d'un classeur qui portent une image.
///
/// Le filtre reprend celui du Python — au moins l'une des deux colonnes
/// renseignée — et rien de plus : c'est [`planifier`] qui décide ensuite ce
/// qui est exploitable.
pub fn lignes(conn: &Connection) -> Result<Vec<LigneImage>> {
    let mut requete = conn.prepare(
        "SELECT card_image_url, card_image_id FROM cards \
         WHERE card_image_url IS NOT NULL OR card_image_id IS NOT NULL",
    )?;
    let lignes = requete
        .query_map([], |l| {
            Ok(LigneImage {
                card_image_url: l.get(0)?,
                card_image_id: l.get(1)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(lignes)
}

/// L'URL Yugipedia d'un identifiant YGOPRODeck, cherchée dans `cardinfo.db`.
///
/// Rend `None` quand la base est absente, quand l'identifiant est inconnu, ou
/// quand la colonne est vide — trois cas où le Python n'a pas de repli non
/// plus.
#[must_use]
pub fn chercher_repli(cardinfo: Option<&Connection>, id: i64) -> Option<String> {
    let conn = cardinfo?;
    let url: Option<String> = conn
        .query_row(
            "SELECT card_url FROM card_images WHERE ygoprodeck_image_id = ?1 LIMIT 1",
            [id],
            |l| l.get(0),
        )
        .ok()?;
    url.filter(|u| !u.is_empty())
}

/// Ce qu'une passe a produit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Issue {
    /// Lignes du classeur portant une image.
    pub lignes: usize,
    /// Images qu'il fallait chercher.
    pub a_telecharger: usize,
    /// Le résultat du téléchargement.
    pub bilan: Bilan,
}

/// Décide ce qu'il y a à télécharger pour un classeur, **sans réseau**.
///
/// Sortie : les cibles et le nombre d'images déjà présentes. C'est la forme
/// que la commande emploie pour son mode « analyse seule », et celle que
/// l'interface emploiera pour ne pas ouvrir de tâche quand il n'y a rien à
/// faire.
pub fn a_faire(
    paths: &Paths,
    code: &str,
    source: SourceImage,
    telechargeur: &Telechargeur<'_>,
) -> Result<(Vec<ygo_images::Cible>, usize, usize)> {
    let chemin = paths.classeur_db(code);
    let conn = ygo_db::connexion::ouvrir_lecture_seule(&chemin)?;
    let lignes = lignes(&conn)?;

    let cardinfo = ygo_db::connexion::ouvrir_lecture_seule(paths.cardinfo_db()).ok();
    let dossier_classeur = paths.img().join(code);

    let (cibles, deja) = planifier(
        &lignes,
        source,
        &paths.img_small(),
        &dossier_classeur,
        |id| chercher_repli(cardinfo.as_ref(), id),
        |chemin: &Path| etat(chemin, telechargeur),
    );
    Ok((cibles, deja, lignes.len()))
}

/// L'état d'un fichier de destination, du point de vue du plan.
fn etat(chemin: &Path, telechargeur: &Telechargeur<'_>) -> Retour {
    if !chemin.exists() {
        return Retour::Absente;
    }
    if telechargeur.est_substitut(chemin) {
        Retour::AReprendre
    } else {
        Retour::Presente
    }
}

/// La passe complète : plan puis téléchargement.
///
/// Le journal est tenu par l'appelant — c'est lui qui sait si la passe fait
/// partie d'une reprise ou d'une ouverture de classeur.
pub async fn passe<F>(
    paths: &Paths,
    client: &ygo_sources::ClientHttp,
    code: &str,
    source: SourceImage,
    progression: F,
) -> Result<Issue>
where
    F: FnMut(usize, usize),
{
    let telechargeur = Telechargeur::nouveau(client, &paths.image_par_defaut());
    let (cibles, _deja, nb_lignes) = a_faire(paths, code, source, &telechargeur)?;
    let a_telecharger = cibles.len();
    let bilan = telechargeur.toutes(&cibles, progression).await;
    crate::replis::consigner(paths, &telechargeur.replis());
    Ok(Issue {
        lignes: nb_lignes,
        a_telecharger,
        bilan,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    fn cardinfo() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE card_images (uuid TEXT, card_uuid TEXT, \
             ygoprodeck_image_id INTEGER, art_url TEXT, card_url TEXT)",
            (),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO card_images VALUES ('u', 'c', 12345, '', \
             'https://ms.yugipedia.com//a/b/Carte.png')",
            (),
        )
        .unwrap();
        conn.execute("INSERT INTO card_images VALUES ('v', 'd', 999, '', '')", ())
            .unwrap();
        conn
    }

    #[test]
    fn le_repli_se_trouve_dans_cardinfo() {
        let conn = cardinfo();
        assert_eq!(
            chercher_repli(Some(&conn), 12345).as_deref(),
            Some("https://ms.yugipedia.com//a/b/Carte.png")
        );
    }

    /// Trois façons de ne pas avoir de repli, et aucune ne doit lever.
    #[test]
    fn sans_repli_la_recherche_rend_none_sans_lever() {
        let conn = cardinfo();
        assert_eq!(chercher_repli(Some(&conn), 42), None, "identifiant inconnu");
        assert_eq!(chercher_repli(Some(&conn), 999), None, "colonne vide");
        assert_eq!(chercher_repli(None, 12345), None, "cardinfo absente");
    }

    #[test]
    fn les_lignes_sans_image_sont_ecartees_des_la_lecture() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE cards (card_image_url TEXT, card_image_id INTEGER)",
            (),
        )
        .unwrap();
        conn.execute("INSERT INTO cards VALUES ('https://x/1.jpg', 1)", ())
            .unwrap();
        conn.execute("INSERT INTO cards VALUES (NULL, 2)", ())
            .unwrap();
        conn.execute("INSERT INTO cards VALUES ('https://x/3.jpg', NULL)", ())
            .unwrap();
        conn.execute("INSERT INTO cards VALUES (NULL, NULL)", ())
            .unwrap();

        let lues = lignes(&conn).unwrap();
        assert_eq!(lues.len(), 3, "la ligne sans rien est écartée");
    }
}
