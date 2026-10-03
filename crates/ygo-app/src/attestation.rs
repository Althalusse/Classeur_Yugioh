// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Ce que la *Set Card List* de Yugipedia dit d'une proposition d'artwork.
//!
//! # Le problème — 2026-09-30
//!
//! Le scan des anomalies ([`crate::anomalies::detecter`]) propose, pour un
//! tirage, **toute** illustration que la carte a connue ailleurs. Il ne sait
//! pas ce que le set a imprimé : Dark Magician a huit illustrations, et le scan
//! les propose toutes partout. Ajoutez les doublons d'identifiant OCG
//! temporaire d'YGOPRODeck — même image, deux numéros — et une bonne part des
//! 379 propositions de l'installation ne sont pas des choix.
//!
//! # La source qui sait
//!
//! La Set list de Yugipedia annote chaque numéro imprimé en illustration
//! alternative ([`ygo_sources::yugipedia::artwork::numeros_illustration_alternative`]).
//! Relevé sur les 11 sets de l'utilisateur qui portent des anomalies : **231
//! propositions attestées, 143 non attestées, 5 inconnues** (page
//! introuvable).
//!
//! # Une première idée, écartée après mesure
//!
//! Juger un tirage d'après le nombre d'images Yugipedia que `cardinfo.db` lui
//! connaît (`set_prints.print_image_url`) aurait écarté 274 propositions sur
//! 379 — dont les sept numéros de `RA02` que la Set list annote « (alternate
//! art) ». Un tirage imprimé **entièrement** en illustration alternative n'a
//! qu'une image, sans suffixe `-AA` : la règle aurait jeté exactement ce
//! qu'elle devait garder.
//!
//! # Ce que l'attestation ne fait pas
//!
//! Elle ne supprime rien : la table `anomalies` reste ce que le scan a trouvé.
//! L'écran masque ce qui n'est pas attesté, et le montre sur demande. Et elle
//! dit **qu'**un numéro porte une illustration alternative, pas **laquelle** :
//! `RA04-EN106` Dark Magician en a cinq — le choix reste à l'utilisateur.

use std::collections::BTreeSet;

use rusqlite::OptionalExtension;
use ygo_core::paths::Paths;

use crate::error::Result;

/// Le verdict de la Set list sur un numéro.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Attestation {
    /// Le numéro est annoncé en illustration alternative.
    Attestee,
    /// La page liste le numéro, sans illustration alternative : le set a
    /// imprimé l'illustration d'origine.
    NonAttestee,
    /// On ne sait pas — page introuvable, réseau absent, numéro absent de la
    /// page. Rien n'est masqué sur la foi d'une ignorance.
    #[default]
    Inconnue,
}

/// Ce qu'une Set list a donné pour un classeur.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SetList {
    /// Le titre de la page lue.
    pub titre: String,
    /// Tous les numéros qu'elle mentionne.
    pub numeros: BTreeSet<String>,
    /// Ceux qu'elle annonce en illustration alternative.
    pub alternatifs: BTreeSet<String>,
}

/// Le verdict pour un numéro — fonction **pure**.
///
/// ```
/// use ygo_app::attestation::{attestation, Attestation, SetList};
/// let liste = SetList {
///     titre: String::new(),
///     numeros: ["RA02-EN006".into(), "RA02-EN047".into()].into(),
///     alternatifs: ["RA02-EN006".into()].into(),
/// };
/// assert_eq!(attestation(Some(&liste), "ra02-en006"), Attestation::Attestee);
/// assert_eq!(attestation(Some(&liste), "RA02-EN047"), Attestation::NonAttestee);
/// // Absent de la page : on ne conclut rien.
/// assert_eq!(attestation(Some(&liste), "RA02-EN999"), Attestation::Inconnue);
/// // Pas de page du tout : pareil.
/// assert_eq!(attestation(None, "RA02-EN006"), Attestation::Inconnue);
/// ```
#[must_use]
pub fn attestation(liste: Option<&SetList>, set_code: &str) -> Attestation {
    let Some(liste) = liste else {
        return Attestation::Inconnue;
    };
    let code = set_code.trim().to_uppercase();
    if liste.alternatifs.contains(&code) {
        Attestation::Attestee
    } else if liste.numeros.contains(&code) {
        Attestation::NonAttestee
    } else {
        Attestation::Inconnue
    }
}

/// Le nom du set et un `set_code` réel d'un classeur — ce qu'il faut pour
/// trouver sa page.
///
/// Le `set_code` vient d'une **ligne** et non du nom de dossier : `LOCR-JP`
/// est un dossier, `LOCR-JP001` un code, et c'est le code qui porte la langue.
///
/// # Errors
///
/// Rend une erreur si le classeur n'est pas lisible.
pub fn reference_du_classeur(paths: &Paths, classeur: &str) -> Result<Option<(String, String)>> {
    let conn = ygo_db::connexion::ouvrir_lecture_seule(paths.classeur_db(classeur))?;
    Ok(conn
        .query_row(
            "SELECT set_name, set_code FROM cards \
              WHERE COALESCE(set_name,'') <> '' AND COALESCE(set_code,'') <> '' \
              ORDER BY rowid LIMIT 1",
            [],
            |l| Ok((l.get::<_, String>(0)?, l.get::<_, String>(1)?)),
        )
        .optional()?)
}

/// Va lire la Set list d'un classeur sur Yugipedia.
///
/// Trois requêtes au plus — la recherche de la page, puis son wikitext —
/// soumises au quota Yugipedia du client (une toutes les 1,1 s). Rend `None`
/// quand la page est introuvable : c'est un verdict « inconnu », pas une
/// erreur.
///
/// # Errors
///
/// Rend une erreur sur une panne réseau ou un classeur illisible.
pub async fn lire_set_list(
    paths: &Paths,
    client: &ygo_sources::ClientHttp,
    classeur: &str,
) -> Result<Option<SetList>> {
    use ygo_sources::yugipedia::{self, artwork, structure};

    let Some((nom_set, code)) = reference_du_classeur(paths, classeur)? else {
        return Ok(None);
    };
    let langue = artwork::langue_set_code(&code);
    let titres = yugipedia::resoudre_pages(client, &nom_set, None).await?;
    let Some(titre) = structure::choisir_page(&titres, &langue) else {
        return Ok(None);
    };
    let Some(wikitext) = artwork::wikitext_page(client, titre).await? else {
        return Ok(None);
    };
    let prefixe = artwork::prefixe_set(&code);
    let numeros = artwork::numeros_de_set_list(&wikitext, &prefixe);
    if numeros.is_empty() {
        // Une page qui ne liste aucun numéro de ce set n'est pas la sienne.
        return Ok(None);
    }
    Ok(Some(SetList {
        titre: titre.to_owned(),
        alternatifs: artwork::numeros_illustration_alternative(&wikitext, &prefixe),
        numeros,
    }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn la_reference_vient_d_une_ligne_nommee() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(tmp.path());
        let chemin = paths.classeur_db("LOCR-JP");
        std::fs::create_dir_all(chemin.parent().unwrap()).unwrap();
        let conn = rusqlite::Connection::open(&chemin).unwrap();
        conn.execute_batch(
            "CREATE TABLE cards (set_name TEXT, set_code TEXT);
             INSERT INTO cards VALUES ('', 'LOCR-JP000');
             INSERT INTO cards VALUES ('Limit Over Collection: The Rivals', 'LOCR-JP001');",
        )
        .unwrap();
        drop(conn);
        assert_eq!(
            reference_du_classeur(&paths, "LOCR-JP").unwrap(),
            Some((
                "Limit Over Collection: The Rivals".to_owned(),
                "LOCR-JP001".to_owned()
            ))
        );
    }

    #[test]
    fn un_classeur_sans_ligne_nommee_n_a_pas_de_reference() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(tmp.path());
        let chemin = paths.classeur_db("X");
        std::fs::create_dir_all(chemin.parent().unwrap()).unwrap();
        let conn = rusqlite::Connection::open(&chemin).unwrap();
        conn.execute_batch("CREATE TABLE cards (set_name TEXT, set_code TEXT);")
            .unwrap();
        drop(conn);
        assert_eq!(reference_du_classeur(&paths, "X").unwrap(), None);
    }
}
