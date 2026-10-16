// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Le contrôle complet — tous les garde-fous des classeurs, en un geste.
//!
//! # Pourquoi — 2026-10-11
//!
//! Chaque défaut corrigé depuis la V1 a laissé derrière lui une réparation :
//! `ygo-cli doublons`, `raretes`, `noms-fr`, `numeros-absents`,
//! `images-tirage`, la reprise des replis. Les mises à jour en déclenchent
//! une partie, la ligne de commande toutes — mais un utilisateur ordinaire
//! n'ouvre pas de terminal, et ne sait pas lesquelles s'appliquent à lui. Le
//! bouton « Contrôle complet » des Options les enchaîne toutes.
//!
//! # L'ordre compte
//!
//! 1. **Sauvegarde** des classeurs et des réglages dans
//!    `backups/avant_controle_<horodatage>/`. Le contrôle supprime des lignes
//!    (étape 2) : rien ne part sans copie. Sans copie, rien ne se fait.
//! 2. **Doublons** — avant les raretés, jamais après : canoniser d'abord rend
//!    les deux lignes canoniques, et le dédoublonnage n'a plus de prise
//!    (mesuré sur `SDWD`, 2026-09-01). Une ligne qui porte une quantité n'est
//!    jamais supprimée.
//! 3. **Raretés** — libellés ramenés à leur forme canonique.
//! 4. **Noms FR officiels** — posés, jamais inventés.
//! 5. **Numéros perdus et Overframe** ([`crate::numeros_absents::partout`]) —
//!    Yugipedia, sur les seuls classeurs qui en ont besoin.
//! 6. **Images de tirage** ([`crate::images_tirage::installation`]) — sans
//!    réseau.
//! 7. **Replis d'images** ([`crate::replis::reprendre`]) — ce que Yugipedia
//!    a ajouté depuis.
//!
//! Une étape en échec est **dite** dans le bilan et n'arrête pas les
//! suivantes : un Yugipedia injoignable n'empêche pas de canoniser une
//! rareté.

use std::collections::BTreeSet;
use std::path::PathBuf;

use ygo_core::paths::Paths;
use ygo_core::rarity::Priorites;

use crate::error::Result;

/// Ce que le contrôle a trouvé et corrigé.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bilan {
    /// Où les classeurs ont été copiés avant le contrôle.
    pub sauvegarde: PathBuf,
    /// Classeurs examinés.
    pub classeurs: usize,
    /// Lignes en double supprimées.
    pub doublons: usize,
    /// Lignes en double gardées parce qu'elles portent une quantité.
    pub doublons_bloques: usize,
    /// Libellés de rareté canonisés.
    pub raretes: usize,
    /// Noms FR officiels posés ou corrigés.
    pub noms_fr: usize,
    /// Numéros perdus par la base, ajoutés.
    pub numeros: usize,
    /// Numéros que la base ne connaît pas du tout — laissés de côté.
    pub introuvables: usize,
    /// Lignes qui ont reçu leur image : Overframe, et cartes sans image (repli).
    pub illustrations: usize,
    /// Lignes qui ont reçu l'image de leur tirage.
    pub images_tirage: usize,
    /// Adresses d'image reposées par la reprise des replis.
    pub replis: usize,
    /// Ce qui n'a pas pu être fait, une phrase par étape.
    pub echecs: Vec<String>,
    /// Classeurs dont les images sont à revoir.
    pub touches: Vec<String>,
}

impl Bilan {
    /// Le nombre total de corrections.
    #[must_use]
    pub fn corrections(&self) -> usize {
        self.doublons
            + self.raretes
            + self.noms_fr
            + self.numeros
            + self.illustrations
            + self.images_tirage
            + self.replis
    }
}

/// Les étapes 2 et 3 — doublons puis raretés —, classeur par classeur.
///
/// Sans réseau. Sans `rarity_config.json`, les doublons ne sont **pas**
/// traités : aucune ligne ne peut alors être déclarée canonique, et
/// supprimer à l'aveugle est hors de question (même règle que
/// `ygo-cli doublons`).
pub fn doublons_et_raretes(paths: &Paths, reference: &Priorites, bilan: &mut Bilan) {
    if reference.is_empty() {
        bilan.echecs.push(
            "doublons : rarity_config.json vide ou illisible — aucune ligne supprimée".to_owned(),
        );
    }
    for code in paths.classeurs_existants() {
        bilan.classeurs += 1;
        let resultat = ygo_db::connexion::ouvrir(paths.classeur_db(&code))
            .map_err(crate::error::AppError::from)
            .and_then(|mut conn| {
                let mut supprimees = 0;
                let mut bloquees = 0;
                if !reference.is_empty() {
                    let r = crate::doublons::analyser(&conn, reference)?;
                    bloquees = r.bloquees.len();
                    if !r.vide() {
                        supprimees = crate::doublons::appliquer(&mut conn, &r)?;
                    }
                }
                let r = crate::raretes::analyser(&conn, reference)?;
                let canonisees = if r.vide() {
                    0
                } else {
                    crate::raretes::appliquer(&mut conn, &r)?
                };
                Ok((supprimees, bloquees, canonisees))
            });
        match resultat {
            Ok((supprimees, bloquees, canonisees)) => {
                bilan.doublons += supprimees;
                bilan.doublons_bloques += bloquees;
                bilan.raretes += canonisees;
                if supprimees + canonisees > 0 {
                    tracing::info!(classeur = %code, doublons = supprimees,
                        raretes = canonisees, "contrôle : classeur corrigé");
                }
            }
            Err(e) => {
                tracing::warn!(classeur = %code, erreur = %e, "contrôle : doublons/raretés");
                bilan
                    .echecs
                    .push(format!("{code} : doublons/raretés — {e}"));
            }
        }
    }
}

/// Le contrôle complet, dans l'ordre de la tête de module.
///
/// # Errors
///
/// Rend une erreur seulement si la sauvegarde échoue : dans ce cas, **rien**
/// n'est touché.
pub async fn complet(
    paths: &Paths,
    client: &ygo_sources::ClientHttp,
    reference: &Priorites,
) -> Result<Bilan> {
    let horodatage = crate::suppression::horodatage_maintenant();
    let sauvegarde =
        crate::maj_appli::sauvegarder_dans(paths, &format!("avant_controle_{horodatage}"))?;
    tracing::info!(sauvegarde = %sauvegarde.display(), "contrôle complet — début");
    let mut bilan = Bilan {
        sauvegarde,
        ..Bilan::default()
    };
    let mut touches: BTreeSet<String> = BTreeSet::new();

    doublons_et_raretes(paths, reference, &mut bilan);

    match crate::noms_fr::rafraichir(paths) {
        Ok(b) => bilan.noms_fr = b.ajoutes + b.corriges,
        Err(e) => bilan.echecs.push(format!("noms FR — {e}")),
    }

    let partout = crate::numeros_absents::partout(paths, client, reference).await;
    bilan.numeros = partout.numeros;
    bilan.introuvables = partout.introuvables;
    bilan.illustrations = partout.illustrations;
    if partout.echecs > 0 {
        bilan.echecs.push(format!(
            "Yugipedia : {} classeur(s) non vérifié(s) — à relancer plus tard",
            partout.echecs
        ));
    }
    touches.extend(partout.touches);

    match crate::images_tirage::installation(paths, reference, true) {
        Ok(b) => {
            bilan.images_tirage = b.a_poser;
            touches.extend(b.touches);
        }
        Err(e) => bilan.echecs.push(format!("images de tirage — {e}")),
    }

    match crate::replis::reprendre(paths, client).await {
        Ok(r) => bilan.replis = r.adresses_reposees,
        Err(e) => bilan.echecs.push(format!("replis d'images — {e}")),
    }

    bilan.touches = touches.into_iter().collect();
    tracing::info!(
        classeurs = bilan.classeurs,
        doublons = bilan.doublons,
        doublons_bloques = bilan.doublons_bloques,
        raretes = bilan.raretes,
        noms_fr = bilan.noms_fr,
        numeros = bilan.numeros,
        illustrations = bilan.illustrations,
        images_tirage = bilan.images_tirage,
        replis = bilan.replis,
        echecs = bilan.echecs.len(),
        "contrôle complet — terminé"
    );
    Ok(bilan)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    fn options() -> Priorites {
        Priorites::depuis_paires([("Common", 1), ("Ultra Rare", 4), ("Secret Rare", 5)])
    }

    /// Une installation d'un classeur, au schéma réel.
    fn installation(lignes: &[(i64, &str, &str, i64, i64)]) -> (tempfile::TempDir, Paths) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(tmp.path());
        paths.creer_dossiers().unwrap();
        let chemin = paths.classeur_db("SDWD");
        std::fs::create_dir_all(chemin.parent().unwrap()).unwrap();
        let conn = ygo_db::connexion::ouvrir(&chemin).unwrap();
        ygo_db::schema::creer_tout(&conn, ygo_db::schema::DDL_CLASSEUR).unwrap();
        for (rowid, set_code, rarity, possede, quantite) in lignes {
            conn.execute(
                "INSERT INTO cards (rowid, card_uuid, name, set_code, rarity, set_name, \
                     possessed, quantite, extended_art) \
                 VALUES (?1, 'u', 'Blue-Eyes White Dragon', ?2, ?3, 'Le Set', ?4, ?5, 0)",
                rusqlite::params![rowid, set_code, rarity, possede, quantite],
            )
            .unwrap();
        }
        (tmp, paths)
    }

    fn raretes(paths: &Paths) -> Vec<(i64, String)> {
        let conn = ygo_db::connexion::ouvrir_lecture_seule(paths.classeur_db("SDWD")).unwrap();
        let mut r = conn
            .prepare("SELECT rowid, rarity FROM cards ORDER BY rowid")
            .unwrap();
        r.query_map([], |l| Ok((l.get(0)?, l.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    }

    /// Le cas `SDWD` réel : un `qty` pris pour une rareté (« 3 »), à côté de
    /// sa ligne canonique. Le doublon part, la ligne possédée reste, et une
    /// abréviation seule est canonisée — dans cet ordre.
    #[test]
    fn les_doublons_partent_avant_que_les_raretes_soient_canonisees() {
        let (_tmp, paths) = installation(&[
            (1, "SDWD-EN013", "Common", 1, 3),
            (2, "SDWD-EN013", "3", 0, 0),
            (3, "SDWD-EN020", "UR", 1, 1),
        ]);
        let mut bilan = Bilan::default();
        doublons_et_raretes(&paths, &options(), &mut bilan);
        assert_eq!(bilan.classeurs, 1);
        assert_eq!(bilan.doublons, 1, "{bilan:?}");
        assert_eq!(bilan.raretes, 1, "{bilan:?}");
        assert!(bilan.echecs.is_empty(), "{:?}", bilan.echecs);
        assert_eq!(
            raretes(&paths),
            vec![(1, "Common".to_owned()), (3, "Ultra Rare".to_owned())]
        );
    }

    /// Un doublon qui porte une quantité n'est jamais supprimé.
    #[test]
    fn un_doublon_possede_reste_en_place() {
        let (_tmp, paths) = installation(&[
            (1, "SDWD-EN013", "Common", 1, 3),
            (2, "SDWD-EN013", "3", 1, 1),
        ]);
        let mut bilan = Bilan::default();
        doublons_et_raretes(&paths, &options(), &mut bilan);
        assert_eq!(bilan.doublons, 0);
        assert_eq!(bilan.doublons_bloques, 1);
        assert_eq!(raretes(&paths).len(), 2, "rien n'est supprimé");
    }

    /// Sans la liste des raretés, aucune ligne n'est supprimée, et le bilan
    /// le dit.
    #[test]
    fn sans_reference_rien_n_est_supprime() {
        let (_tmp, paths) = installation(&[
            (1, "SDWD-EN013", "Common", 1, 3),
            (2, "SDWD-EN013", "3", 0, 0),
        ]);
        let mut bilan = Bilan::default();
        doublons_et_raretes(
            &paths,
            &Priorites::depuis_paires(Vec::<(&str, i64)>::new()),
            &mut bilan,
        );
        assert_eq!(bilan.doublons, 0);
        assert_eq!(raretes(&paths).len(), 2);
        assert!(
            bilan.echecs[0].contains("rarity_config.json"),
            "{:?}",
            bilan.echecs
        );
    }

    /// La sauvegarde copie les classeurs avant tout.
    #[test]
    fn la_sauvegarde_copie_les_classeurs() {
        let (_tmp, paths) = installation(&[(1, "SDWD-EN013", "Common", 1, 3)]);
        let dossier = crate::maj_appli::sauvegarder_dans(&paths, "avant_controle_essai").unwrap();
        assert!(dossier
            .join("classeur_creer")
            .join("SDWD")
            .join("SDWD.db")
            .is_file());
    }
}
