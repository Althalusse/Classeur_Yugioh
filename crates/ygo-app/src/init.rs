// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Initialisation de la base de référence.
//!
//! Portage de `module/BDD_creation.run_init`.
//!
//! # Le flux, et ce qui a changé
//!
//! ```text
//!  1. archive YGOJSON            → streamée sur DISQUE      ← correctif du crash
//!  2. catalogue YGOPRODeck       → facultatif
//!  3. parsing des cartes         → pur
//!  4. parsing des sets           → pur
//!  5. résolution + expansion des artworks
//!  6. enrichissement des statistiques
//!  7. schéma + insertions + index → UNE transaction
//!  8. vérification
//! ```
//!
//! Trois différences avec le Python, toutes voulues :
//!
//! - l'archive ne passe jamais par la mémoire (§ [`ygo_sources::ygojson`]) ;
//! - la base est construite dans un fichier **temporaire** puis déplacée à sa
//!   place, ce qui donne l'atomicité du §3.1 sans faire porter 170 Mo au
//!   journal WAL d'une base en service (cf. [`Options::via_fichier_temporaire`]) ;
//! - la progression passe par un canal, pas par un rappel (règle R6).

use std::path::{Path, PathBuf};

use tokio::sync::mpsc;
use ygo_core::modele::{CartesParsees, SetsParses, TirageBrut};
use ygo_core::paths::Paths;
use ygo_db::connexion::Bases;
use ygo_db::init::Comptages;
use ygo_sources::{ygojson, ygoprodeck, ClientHttp};

use crate::error::{AppError, Result};

/// Étape franchie par l'initialisation.
///
/// Remplace le `log(message, couleur)` du Python : une valeur typée, émise dans
/// un canal, que l'interface met en forme comme elle l'entend. Le crate ne sait
/// pas qu'une interface existe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Etape {
    /// Téléchargement de l'archive : octets reçus, taille totale si connue.
    TelechargementArchive {
        /// Octets déjà écrits sur disque.
        recus: u64,
        /// Taille annoncée par le serveur.
        total: Option<u64>,
    },
    /// Archive téléchargée.
    ArchiveRecue {
        /// Taille du fichier.
        octets: u64,
    },
    /// Catalogue YGOPRODeck reçu.
    CatalogueRecu {
        /// Nombre d'entrées.
        entrees: usize,
    },
    /// YGOPRODeck injoignable — les statistiques manqueront.
    CatalogueIndisponible,
    /// Cartes parsées.
    CartesParsees {
        /// Cartes.
        cartes: usize,
        /// Textes.
        textes: usize,
        /// Illustrations.
        images: usize,
    },
    /// Sets parsés.
    SetsParses {
        /// Sets.
        sets: usize,
        /// Déclinaisons locales.
        locales: usize,
        /// Tirages avant expansion.
        tirages: usize,
    },
    /// Expansion multi-artworks terminée.
    ArtworksResolus {
        /// Tirages ajoutés par l'expansion.
        ajoutes: usize,
    },
    /// Enrichissement terminé.
    CartesEnrichies {
        /// Cartes dotées de statistiques.
        enrichies: usize,
    },
    /// Écriture en base en cours.
    EcritureBase,
    /// Base construite et vérifiée.
    Terminee(Comptages),
}

/// Canal de progression.
pub type Progression = mpsc::UnboundedSender<Etape>;

/// D'où viennent les données.
#[derive(Debug, Clone)]
pub enum Source {
    /// Téléchargement réel depuis GitHub et YGOPRODeck.
    Reseau,
    /// Archive déjà sur disque, catalogue YGOPRODeck facultatif.
    ///
    /// C'est le mode de test hors ligne : il permet de rejouer une
    /// initialisation entière sur des fixtures figées, donc de comparer le
    /// résultat à celui de la V1.0.4 sans dépendre d'un wiki vivant.
    Locale {
        /// Chemin de `aggregate.zip`.
        archive: PathBuf,
        /// Chemin d'une réponse `cardinfo.php` enregistrée.
        catalogue: Option<PathBuf>,
    },
}

/// Réglages de l'initialisation.
#[derive(Debug, Clone)]
pub struct Options {
    /// Provenance des données.
    pub source: Source,
    /// Construire dans un fichier temporaire, puis remplacer la base en place.
    ///
    /// Activé par défaut. L'invariant du §3.1 — « pas de base à moitié
    /// peuplée » — est ainsi obtenu **sans** faire porter les 170 Mo
    /// d'insertions au journal WAL d'un fichier que l'application peut déjà
    /// avoir ouvert. Le remplacement final est une opération du système de
    /// fichiers, quasi instantanée.
    pub via_fichier_temporaire: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            source: Source::Reseau,
            via_fichier_temporaire: true,
        }
    }
}

impl Options {
    /// Initialisation hors ligne depuis une archive locale.
    pub fn hors_ligne(archive: impl Into<PathBuf>) -> Self {
        Self {
            source: Source::Locale {
                archive: archive.into(),
                catalogue: None,
            },
            ..Self::default()
        }
    }

    /// Ajoute un catalogue YGOPRODeck enregistré.
    pub fn avec_catalogue(mut self, chemin: impl Into<PathBuf>) -> Self {
        if let Source::Locale { catalogue, .. } = &mut self.source {
            *catalogue = Some(chemin.into());
        }
        self
    }
}

fn signaler(progression: Option<&Progression>, etape: Etape) {
    // Journalisée AVANT d'être envoyée, et quel que soit l'écouteur : c'est ce
    // qui rend l'initialisation consultable après coup. Lancée par
    // `Lancer-Appli.ps1`, l'application n'a pas de console — sans cette trace,
    // une mise à jour qui s'arrête en route ne laisse rien à examiner.
    journaliser(&etape);
    if let Some(canal) = progression {
        // Un récepteur fermé n'est pas une erreur : personne n'écoute, on
        // continue. C'est tout l'intérêt d'un canal plutôt que d'un rappel.
        let _ = canal.send(etape);
    }
}

/// Une ligne de journal par étape franchie.
///
/// # Ce qui n'est PAS journalisé ici, et pourquoi
///
/// - `TelechargementArchive` : émise à chaque bloc reçu, soit des centaines de
///   fois. `ygojson::telecharger_archive` la résume une fois, à l'arrivée.
/// - `ArchiveRecue`, `CatalogueRecu`, `Terminee` : la couche du dessous les
///   journalise **déjà**, et avec plus de détail — le chemin du fichier
///   temporaire, les comptages table par table. Les répéter donnait deux
///   lignes identiques à la seconde près, relevé sur le journal réel du
///   2026-09-05.
///
/// Le partage est celui-ci : la couche basse dit ce qu'elle a obtenu, cette
/// couche dit où en est le pipeline. Une étape déjà couverte par la première
/// n'a rien à ajouter.
fn journaliser(etape: &Etape) {
    match etape {
        Etape::TelechargementArchive { .. }
        | Etape::ArchiveRecue { .. }
        | Etape::CatalogueRecu { .. }
        | Etape::Terminee(_) => {}
        Etape::CatalogueIndisponible => {
            tracing::warn!("YGOPRODeck indisponible — pas de statistiques");
        }
        Etape::CartesParsees {
            cartes,
            textes,
            images,
        } => tracing::info!(cartes, textes, images, "cartes parsées"),
        Etape::SetsParses {
            sets,
            locales,
            tirages,
        } => tracing::info!(sets, locales, tirages, "sets parsés"),
        Etape::ArtworksResolus { ajoutes } => tracing::info!(ajoutes, "artworks étendus"),
        Etape::CartesEnrichies { enrichies } => tracing::info!(enrichies, "cartes enrichies"),
        Etape::EcritureBase => tracing::info!("écriture en base"),
    }
}

/// Initialise `bdd/cardinfo.db` pour une installation.
///
/// Retourne les comptages, qui sont les valeurs à comparer avec celles de la
/// V1.0.4 pour valider le portage.
pub async fn initialiser(
    paths: &Paths,
    options: &Options,
    progression: Option<&Progression>,
) -> Result<Comptages> {
    paths.creer_dossiers()?;
    tracing::info!(
        installation = %paths.racine().display(),
        source = ?options.source,
        "initialisation de la base — début"
    );

    // ── 1 et 2 : acquisition ────────────────────────────────────────────────
    // Chaque `?` de cette fonction passe par `echec` : une initialisation
    // interrompue laissait sinon un journal qui s'arrête net, sans dire à
    // quelle étape ni pourquoi. C'est le cas le plus fréquent — archive
    // absente, réseau coupé — et c'était le moins renseigné.
    let dossier_travail = tempfile::tempdir().map_err(|e| echec("dossier temporaire", e.into()))?;
    let (archive, catalogue) = acquerir(options, dossier_travail.path(), progression)
        .await
        .map_err(|e| echec("acquisition", e))?;

    // ── 3 et 4 : parsing, pur ───────────────────────────────────────────────
    let (cartes, mut sets) =
        ygojson::charger_depuis_archive(&archive).map_err(|e| echec("parsing", e.into()))?;
    if cartes.cartes.is_empty() {
        return Err(echec("parsing", AppError::YgojsonVide));
    }
    signaler(
        progression,
        Etape::CartesParsees {
            cartes: cartes.cartes.len(),
            textes: cartes.textes.len(),
            images: cartes.images.len(),
        },
    );
    signaler(
        progression,
        Etape::SetsParses {
            sets: sets.sets.len(),
            locales: sets.locales.len(),
            tirages: sets.tirages.len(),
        },
    );

    // ── 5 et 6 : artworks puis statistiques ─────────────────────────────────
    // Les tirages sont extraits de `sets` plutôt que clonés : ils sont plus de
    // 320 000, et chacun porte six chaînes. Le clone coûtait à lui seul plus
    // que l'archive entière.
    let tirages_bruts = std::mem::take(&mut sets.tirages);
    let (cartes, tirages) = appliquer_ygoprodeck(cartes, tirages_bruts, &catalogue, progression);

    // ── 7 : écriture, en une transaction ────────────────────────────────────
    signaler(progression, Etape::EcritureBase);
    let definitive = paths.cardinfo_db();
    let comptages = ecrire(&definitive, &cartes, &sets, &tirages, options)
        .map_err(|e| echec("écriture en base", e))?;

    // ── 8 : vérification ────────────────────────────────────────────────────
    ygo_db::init::verifier(&definitive).map_err(|e| echec("vérification", e.into()))?;

    // ── 9 : priorités de rareté ─────────────────────────────────────────────
    // Ici, et pas dans `maj` : le mode hors ligne du `ygo-cli` n'y passe pas,
    // et une base construite sans priorités est une base dont le tri par
    // rareté ne range rien. Un échec d'écriture ne perd pas la base — il se
    // journalise.
    if let Err(e) = crate::raretes::synchroniser_priorites(paths) {
        tracing::warn!(erreur = %e, "priorités de rareté non écrites");
    }
    signaler(progression, Etape::Terminee(comptages));
    Ok(comptages)
}

/// Journalise un échec en nommant l'étape, et rend l'erreur inchangée.
///
/// Le message d'erreur seul ne dit pas *où* : « fichier introuvable » peut
/// venir de l'archive comme du dossier temporaire.
fn echec(etape: &str, erreur: AppError) -> AppError {
    tracing::error!(etape, erreur = %erreur, "initialisation de la base — échec");
    erreur
}

/// Récupère l'archive et le catalogue, par le réseau ou depuis le disque.
async fn acquerir(
    options: &Options,
    dossier: &Path,
    progression: Option<&Progression>,
) -> Result<(PathBuf, Vec<ygoprodeck::CarteYgoprodeck>)> {
    match &options.source {
        Source::Reseau => {
            let client = ClientHttp::new()?;

            let archive = dossier.join("aggregate.zip");
            let rapport = |recus: u64, total: Option<u64>| {
                signaler(progression, Etape::TelechargementArchive { recus, total });
            };
            let octets = ygojson::telecharger_archive(&client, &archive, Some(&rapport)).await?;
            signaler(progression, Etape::ArchiveRecue { octets });

            // YGOPRODeck est facultatif : son échec dégrade, il ne bloque pas.
            let catalogue = match ygoprodeck::telecharger_catalogue(&client).await {
                Ok(c) => {
                    signaler(progression, Etape::CatalogueRecu { entrees: c.len() });
                    c
                }
                Err(e) => {
                    tracing::warn!(erreur = %e, "YGOPRODeck indisponible — statistiques absentes");
                    signaler(progression, Etape::CatalogueIndisponible);
                    Vec::new()
                }
            };
            Ok((archive, catalogue))
        }

        Source::Locale { archive, catalogue } => {
            let octets = ygojson::taille_fichier(archive)?;
            signaler(progression, Etape::ArchiveRecue { octets });

            let cartes = match catalogue {
                Some(chemin) => {
                    let texte = std::fs::read(chemin)?;
                    let reponse: ygoprodeck::ReponseCatalogue = serde_json::from_slice(&texte)
                        .map_err(|e| {
                            ygo_sources::SourceError::deserialisation(
                                chemin.display().to_string(),
                                e,
                            )
                        })?;
                    signaler(
                        progression,
                        Etape::CatalogueRecu {
                            entrees: reponse.data.len(),
                        },
                    );
                    reponse.data
                }
                None => {
                    signaler(progression, Etape::CatalogueIndisponible);
                    Vec::new()
                }
            };
            Ok((archive.clone(), cartes))
        }
    }
}

/// Applique la passe YGOPRODeck : artworks d'abord, statistiques ensuite.
///
/// L'ordre compte et il est celui du Python : l'expansion multi-artworks lit
/// `images_rows`, que l'enrichissement ne modifie pas — mais l'inverse ne
/// serait pas vrai si un jour l'enrichissement touchait aux illustrations.
fn appliquer_ygoprodeck(
    mut cartes: CartesParsees,
    tirages_bruts: Vec<TirageBrut>,
    catalogue: &[ygoprodeck::CarteYgoprodeck],
    progression: Option<&Progression>,
) -> (CartesParsees, Vec<TirageBrut>) {
    if catalogue.is_empty() {
        return (cartes, tirages_bruts);
    }

    let avant = tirages_bruts.len();
    let tirages =
        ygoprodeck::resoudre_et_etendre_artworks(tirages_bruts, catalogue, &cartes.images);
    signaler(
        progression,
        Etape::ArtworksResolus {
            ajoutes: tirages.len().saturating_sub(avant),
        },
    );

    let index = ygoprodeck::indexer_par_password(catalogue);
    let enrichies = ygoprodeck::enrichir_cartes(&mut cartes.cartes, &index);
    signaler(progression, Etape::CartesEnrichies { enrichies });

    (cartes, tirages)
}

/// Écrit la base, éventuellement via un fichier temporaire.
fn ecrire(
    definitive: &Path,
    cartes: &CartesParsees,
    sets: &SetsParses,
    tirages: &[TirageBrut],
    options: &Options,
) -> Result<Comptages> {
    let bases = Bases::new();

    if !options.via_fichier_temporaire {
        return Ok(bases.avec_base(definitive, |tx| {
            ygo_db::init::construire(tx, cartes, sets, tirages)
        })?);
    }

    // Construction à côté de la base définitive — même système de fichiers,
    // donc le remplacement final est un simple renommage.
    let dossier = definitive.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dossier)?;
    let provisoire = dossier.join("cardinfo.db.nouvelle");
    let _ = std::fs::remove_file(&provisoire);

    // `cards_overrides` doit survivre : on la reprend depuis la base en place
    // avant de basculer. Elle fait partie du schéma d'init, donc la nouvelle
    // base la porte déjà — vide ; il faut la remplir.
    //
    // Les TROIS AUTRES tables préservées, elles, n'appartiennent pas au schéma
    // d'init : la nouvelle base ne les a pas du tout. Elles se reprennent après
    // construction, avec leur DDL — cf. `reprendre_tables_hors_init`.
    let overrides = lire_overrides(definitive)?;

    let comptages = bases.avec_base(&provisoire, |tx| {
        let c = ygo_db::init::construire(tx, cartes, sets, tirages)?;
        restaurer_overrides(tx, &overrides)?;
        Ok(c)
    })?;

    // Les tables hors périmètre d'init — `anomalies`, `overframe_sync`,
    // `card_images_externes` — se reprennent ICI, sur la base provisoire et
    // avant la bascule : `ATTACH` ne peut pas vivre dans la transaction de
    // construction.
    //
    // Leur perte ne se voyait pas. Elle a été constatée le 2026-09-05 sur
    // l'installation réelle, une mise à jour plus tard : trois tables
    // disparues, dont les 1 613 lignes de `card_images_externes` produites par
    // la V1.0.4. Un échec de reprise n'annule pas la mise à jour — la base
    // neuve est bonne — mais il se dit.
    match ygo_db::init::reprendre_tables_hors_init(&provisoire, definitive) {
        Ok(reprises) if !reprises.is_empty() => {
            for (table, lignes) in &reprises {
                tracing::info!(table = %table, lignes, "table hors init reprise");
            }
        }
        Ok(_) => {}
        Err(e) => tracing::warn!(erreur = %e, "tables hors init NON reprises"),
    }

    // SQLite laisse des fichiers annexes en mode WAL ; on les écarte avant de
    // renommer, sinon la nouvelle base hériterait d'un journal étranger.
    for suffixe in ["-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffixe}", provisoire.display()));
        let _ = std::fs::remove_file(format!("{}{suffixe}", definitive.display()));
    }
    std::fs::rename(&provisoire, definitive)?;
    tracing::info!(
        base = %definitive.display(),
        overrides = overrides.len(),
        "base remplacée par sa nouvelle version"
    );
    Ok(comptages)
}

/// Une ligne de `cards_overrides`, telle qu'elle doit traverser une
/// réinitialisation.
type LigneOverride = (
    Option<i64>,    // base_card_id
    String,         // name
    Option<i64>,    // card_images_id
    Option<String>, // card_images_image_url
    Option<String>, // card_images_image_url_small
    Option<String>, // card_sets_set_name
    String,         // card_sets_set_code
    String,         // card_sets_set_rarity
    Option<String>, // card_sets_set_rarity_code
    Option<String>, // reason
    Option<String>, // created_at
);

/// Lit `cards_overrides` d'une base existante. Base absente : liste vide.
fn lire_overrides(base: &Path) -> Result<Vec<LigneOverride>> {
    if !base.is_file() {
        return Ok(Vec::new());
    }
    let conn = match ygo_db::connexion::ouvrir_lecture_seule(base) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(erreur = %e, "base existante illisible — aucune correction reprise");
            return Ok(Vec::new());
        }
    };
    let existe: i64 = conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='cards_overrides'",
        [],
        |r| r.get(0),
    )?;
    if existe == 0 {
        return Ok(Vec::new());
    }

    let mut requete = conn.prepare(
        "SELECT base_card_id, name, card_images_id, card_images_image_url, \
                card_images_image_url_small, card_sets_set_name, card_sets_set_code, \
                card_sets_set_rarity, card_sets_set_rarity_code, reason, created_at \
         FROM cards_overrides ORDER BY override_id",
    )?;
    let lignes = requete.query_map([], |r| {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
            r.get(6)?,
            r.get(7)?,
            r.get(8)?,
            r.get(9)?,
            r.get(10)?,
        ))
    })?;
    let mut sortie = Vec::new();
    for ligne in lignes {
        sortie.push(ligne?);
    }
    Ok(sortie)
}

/// Réinsère les corrections manuelles dans la base fraîchement construite.
fn restaurer_overrides(
    tx: &rusqlite::Transaction<'_>,
    lignes: &[LigneOverride],
) -> ygo_db::Result<()> {
    if lignes.is_empty() {
        return Ok(());
    }
    let mut requete = tx.prepare(
        "INSERT OR IGNORE INTO cards_overrides \
         (base_card_id, name, card_images_id, card_images_image_url, \
          card_images_image_url_small, card_sets_set_name, card_sets_set_code, \
          card_sets_set_rarity, card_sets_set_rarity_code, reason, created_at) \
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
    )?;
    for l in lignes {
        requete.execute(rusqlite::params![
            l.0, l.1, l.2, l.3, l.4, l.5, l.6, l.7, l.8, l.9, l.10
        ])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;
    use std::io::Write as _;

    /// Archive minimale au format YGOJSON.
    fn archive(dossier: &Path) -> PathBuf {
        use zip::write::SimpleFileOptions;

        let chemin = dossier.join("aggregate.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&chemin).unwrap());
        let options = SimpleFileOptions::default();

        zip.start_file("individual/cards.json", options).unwrap();
        zip.write_all(
            br#"[
              {"id":"c1","cardType":"monster",
               "text":{"en":{"name":"Blue-Eyes White Dragon"},
                       "fr":{"name":"Dragon Blanc aux Yeux Bleus"}},
               "images":[{"id":"i1","password":89631139,"art":"https://x/a1.jpg"}]},
              {"id":"c2","cardType":"monster",
               "text":{"en":{"name":"Dark Magician"}},
               "images":[{"id":"i2","password":46986414,"art":"https://x/a2.jpg"}]}
            ]"#,
        )
        .unwrap();

        zip.start_file("individual/sets.json", options).unwrap();
        zip.write_all(
            br#"[
              {"id":"s1","name":{"en":"Legend of Blue Eyes"},
               "locales":{"en":{"prefix":"LOB-EN"},"fr":{"prefix":"LOB-FR"}},
               "contents":[{"locales":["en","fr"],"editions":["1st"],"cards":[
                  {"id":"i1","card":"c1","suffix":"001","rarity":"ultra"},
                  {"id":"i2","card":"c2","suffix":"005","rarity":"common"}]}]}
            ]"#,
        )
        .unwrap();
        zip.finish().unwrap();
        chemin
    }

    fn catalogue(dossier: &Path) -> PathBuf {
        let chemin = dossier.join("cardinfo.json");
        std::fs::write(
            &chemin,
            br#"{"data":[
              {"id":89631139,"frameType":"normal","atk":3000,"def":2500,"level":8,
               "attribute":"LIGHT","race":"Dragon",
               "card_sets":[{"set_code":"LOB-EN001","set_rarity":"Ultra Rare"}]},
              {"id":46986414,"frameType":"effect","atk":2500,"def":2100,"level":7,
               "attribute":"DARK","race":"Spellcaster",
               "card_sets":[{"set_code":"LOB-EN005","set_rarity":"Common"}]}
            ]}"#,
        )
        .unwrap();
        chemin
    }

    #[tokio::test]
    async fn initialisation_hors_ligne_complete() {
        let tmp = tempfile::tempdir().unwrap();
        let sources = tmp.path().join("sources");
        std::fs::create_dir_all(&sources).unwrap();
        let paths = Paths::depuis_racine(tmp.path().join("install"));

        let options = Options::hors_ligne(archive(&sources)).avec_catalogue(catalogue(&sources));

        let (tx, mut rx) = mpsc::unbounded_channel();
        let comptages = initialiser(&paths, &options, Some(&tx)).await.unwrap();
        drop(tx);

        assert_eq!(comptages.cartes, 2);
        assert_eq!(comptages.textes, 3, "2 anglais + 1 français");
        assert_eq!(comptages.images, 2);
        assert_eq!(comptages.sets, 1);
        assert_eq!(comptages.locales, 2);
        assert_eq!(comptages.tirages, 4, "2 cartes × 2 locales");
        assert_eq!(
            comptages.sans_traduction_fr, 1,
            "seule c2 n'a pas de nom FR"
        );

        // La base est en place, vérifiée, et le fichier provisoire a disparu.
        assert!(paths.cardinfo_db().is_file());
        assert!(!paths.bdd().join("cardinfo.db.nouvelle").exists());

        // Le canal a bien porté les étapes, dans l'ordre.
        let mut etapes = Vec::new();
        while let Ok(e) = rx.try_recv() {
            etapes.push(e);
        }
        assert!(matches!(etapes.first(), Some(Etape::ArchiveRecue { .. })));
        assert!(matches!(etapes.last(), Some(Etape::Terminee(_))));
        assert!(etapes
            .iter()
            .any(|e| matches!(e, Etape::CartesEnrichies { enrichies: 2 })));
    }

    #[tokio::test]
    async fn sans_catalogue_les_statistiques_manquent_mais_l_init_reussit() {
        let tmp = tempfile::tempdir().unwrap();
        let sources = tmp.path().join("sources");
        std::fs::create_dir_all(&sources).unwrap();
        let paths = Paths::depuis_racine(tmp.path().join("install"));

        let options = Options::hors_ligne(archive(&sources));
        let comptages = initialiser(&paths, &options, None).await.unwrap();

        assert_eq!(comptages.cartes, 2);
        assert_eq!(comptages.tirages, 4);

        let conn = ygo_db::connexion::ouvrir_lecture_seule(paths.cardinfo_db()).unwrap();
        let enrichies: i64 = conn
            .query_row(
                "SELECT count(*) FROM cards WHERE frame_type IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(enrichies, 0, "YGOPRODeck absent : aucune statistique");
    }

    #[tokio::test]
    async fn l_enrichissement_arrive_bien_en_base() {
        let tmp = tempfile::tempdir().unwrap();
        let sources = tmp.path().join("sources");
        std::fs::create_dir_all(&sources).unwrap();
        let paths = Paths::depuis_racine(tmp.path().join("install"));

        let options = Options::hors_ligne(archive(&sources)).avec_catalogue(catalogue(&sources));
        initialiser(&paths, &options, None).await.unwrap();

        let conn = ygo_db::connexion::ouvrir_lecture_seule(paths.cardinfo_db()).unwrap();
        let (atk, def, race): (i64, i64, String) = conn
            .query_row(
                "SELECT atk, \"def\", race FROM cards WHERE ygoprodeck_id = 89631139",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!((atk, def, race.as_str()), (3000, 2500, "Dragon"));
    }

    /// Le fichier temporaire ne doit pas faire perdre les corrections
    /// manuelles : c'est le piège de cette approche, et l'invariant du §3.1.
    #[tokio::test]
    async fn les_corrections_manuelles_traversent_le_remplacement() {
        let tmp = tempfile::tempdir().unwrap();
        let sources = tmp.path().join("sources");
        std::fs::create_dir_all(&sources).unwrap();
        let paths = Paths::depuis_racine(tmp.path().join("install"));
        let options = Options::hors_ligne(archive(&sources));

        initialiser(&paths, &options, None).await.unwrap();

        Bases::new()
            .avec_base(paths.cardinfo_db(), |tx| {
                tx.execute(
                    "INSERT INTO cards_overrides \
                     (base_card_id, name, card_sets_set_code, card_sets_set_rarity, reason) \
                     VALUES (89631139, 'Blue-Eyes White Dragon', 'LOB-EN001', 'Ultra Rare', 'à la main')",
                    [],
                )?;
                Ok(())
            })
            .unwrap();

        // Deuxième initialisation : la base est intégralement remplacée.
        initialiser(&paths, &options, None).await.unwrap();

        let conn = ygo_db::connexion::ouvrir_lecture_seule(paths.cardinfo_db()).unwrap();
        let (n, raison): (i64, String) = conn
            .query_row(
                "SELECT count(*), max(reason) FROM cards_overrides",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(n, 1, "la correction manuelle doit survivre au remplacement");
        assert_eq!(raison, "à la main");
    }

    #[tokio::test]
    async fn deux_initialisations_donnent_le_meme_resultat() {
        let tmp = tempfile::tempdir().unwrap();
        let sources = tmp.path().join("sources");
        std::fs::create_dir_all(&sources).unwrap();
        let paths = Paths::depuis_racine(tmp.path().join("install"));
        let options = Options::hors_ligne(archive(&sources)).avec_catalogue(catalogue(&sources));

        let a = initialiser(&paths, &options, None).await.unwrap();
        let b = initialiser(&paths, &options, None).await.unwrap();
        assert_eq!(a, b, "l'initialisation doit être idempotente");
    }

    #[tokio::test]
    async fn une_archive_sans_carte_est_refusee() {
        use zip::write::SimpleFileOptions;

        let tmp = tempfile::tempdir().unwrap();
        let chemin = tmp.path().join("vide.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&chemin).unwrap());
        zip.start_file("individual/cards.json", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"[]").unwrap();
        zip.finish().unwrap();

        let paths = Paths::depuis_racine(tmp.path().join("install"));
        let erreur = initialiser(&paths, &Options::hors_ligne(&chemin), None)
            .await
            .unwrap_err();
        assert!(matches!(erreur, AppError::YgojsonVide));
        assert!(
            !paths.cardinfo_db().exists(),
            "aucune base ne doit être écrite quand YGOJSON est vide"
        );
    }

    #[tokio::test]
    async fn mode_sans_fichier_temporaire() {
        let tmp = tempfile::tempdir().unwrap();
        let sources = tmp.path().join("sources");
        std::fs::create_dir_all(&sources).unwrap();
        let paths = Paths::depuis_racine(tmp.path().join("install"));

        let mut options = Options::hors_ligne(archive(&sources));
        options.via_fichier_temporaire = false;

        let comptages = initialiser(&paths, &options, None).await.unwrap();
        assert_eq!(comptages.cartes, 2);
        assert!(paths.cardinfo_db().is_file());
    }

    /// Une base construite arrive avec ses priorités de rareté.
    ///
    /// Le mode hors ligne du `ygo-cli` ne passe pas par `maj` : c'est
    /// pourquoi la synchronisation vit ici et non là-bas.
    #[tokio::test]
    async fn une_base_construite_arrive_avec_ses_priorites() {
        let tmp = tempfile::tempdir().unwrap();
        let sources = tmp.path().join("sources");
        std::fs::create_dir_all(&sources).unwrap();
        let paths = Paths::depuis_racine(tmp.path().join("install"));
        assert!(!paths.rarity_config().exists());

        initialiser(&paths, &Options::hors_ligne(archive(&sources)), None)
            .await
            .unwrap();

        assert!(paths.rarity_config().is_file());
        let table = ygo_core::rarity::Priorites::charger(paths.rarity_config());
        assert!(
            !table.is_empty(),
            "sans elles, le tri par rareté ne range rien"
        );
    }

    /// Le défaut du 2026-09-05, éprouvé **par le chemin réellement emprunté**.
    ///
    /// # Pourquoi il avait échappé aux tests
    ///
    /// `ygo-db` en avait un, nommé `les_tables_hors_init_survivent`, et il
    /// passait. Il éprouvait `construire` sur une base **en place** — où
    /// aucune table n'est supprimée, forcément. Le chemin par défaut,
    /// `via_fichier_temporaire: true`, remplace le fichier entier : la
    /// propriété y était fausse, et aucun test ne la regardait là.
    ///
    /// Une assertion vraie sur un chemin que personne n'emprunte ne protège
    /// rien. Celle-ci passe par `initialiser`, options par défaut.
    #[tokio::test]
    async fn une_reinitialisation_garde_les_tables_qu_elle_n_a_pas_creees() {
        let tmp = tempfile::tempdir().unwrap();
        let sources = tmp.path().join("sources");
        std::fs::create_dir_all(&sources).unwrap();
        let paths = Paths::depuis_racine(tmp.path().join("install"));
        let options = Options::hors_ligne(archive(&sources));

        // Première base, puis ce que les autres modules y ajoutent.
        initialiser(&paths, &options, None).await.unwrap();
        {
            let conn = ygo_db::connexion::ouvrir(paths.cardinfo_db()).unwrap();
            conn.execute_batch(
                "CREATE TABLE anomalies (id INTEGER PRIMARY KEY, nom TEXT, corrige INTEGER);
                 CREATE TABLE overframe_sync (set_prefix TEXT PRIMARY KEY, revid TEXT);
                 INSERT INTO anomalies (nom, corrige) VALUES ('Dragon', 1);
                 INSERT INTO overframe_sync VALUES ('LOCR-JP', '5945579');",
            )
            .unwrap();
        }

        // Une mise à jour, par le chemin par défaut : fichier temporaire puis
        // renommage.
        assert!(options.via_fichier_temporaire);
        initialiser(&paths, &options, None).await.unwrap();

        let conn = ygo_db::connexion::ouvrir_lecture_seule(paths.cardinfo_db()).unwrap();
        let corrige: i64 = conn
            .query_row(
                "SELECT corrige FROM anomalies WHERE nom = 'Dragon'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(corrige, 1, "le drapeau `corrige` ne se reconstruit pas");

        let revid: String = conn
            .query_row("SELECT revid FROM overframe_sync", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            revid, "5945579",
            "sans overframe_sync, la passe Overframe se rejouerait indéfiniment"
        );
    }
}
