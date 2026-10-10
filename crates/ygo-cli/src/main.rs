// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! `ygo-cli` — binaire de diagnostic, sans écran.
//!
//! Sa raison d'être : **comparer le Rust au Python sans dépendre de
//! l'interface**. Tant que le lot « interface » n'existe pas, c'est ici qu'on
//! vérifie que le portage produit exactement ce que produisait la V1.0.4.
//!
//! ```text
//! ygo-cli schema <installation>              vérifie les schémas d'une installation
//! ygo-cli config <installation>              affiche les préférences lues par le Rust
//! ygo-cli creer-vide <dossier>               arborescence et bases vides conformes
//! ygo-cli init <installation> [--archive f]  construit cardinfo.db
//! ygo-cli comparer <réference> <candidate>   compare deux cardinfo.db table par table
//! ```
//!
//! `<installation>` est le dossier qui contient `bdd/`, `img/` et `logs/` —
//! typiquement `Projet Python/V1.0.3` ou `Projet Python/V1.0.4`.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{bail, Context};
use ygo_app::accueil;
use ygo_app::anomalies;
use ygo_app::artworks;
use ygo_app::creation;
use ygo_app::doublons;
use ygo_app::images;
use ygo_app::images_tirage;
use ygo_app::init::{Etape, Options, Source};
use ygo_app::overframe;
use ygo_app::raretes;
use ygo_core::config::Config;
use ygo_core::paths::Paths;
use ygo_core::rarity::Priorites;
use ygo_core::version;
use ygo_db::connexion;
use ygo_db::migrations;
use ygo_db::schema::{self, DDL_CARDINFO, DDL_CLASSEUR};

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[tokio::main]
async fn main() -> ExitCode {
    match executer().await {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("erreur : {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// Retourne `Ok(false)` quand la commande a fonctionné mais que sa conclusion
/// est négative (schéma non conforme, par exemple).
async fn executer() -> anyhow::Result<bool> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (commande, reste) = match args.split_first() {
        Some((c, reste)) => (c.as_str(), reste),
        None => {
            aide();
            return Ok(false);
        }
    };
    let cible = reste.first().map(PathBuf::from);

    // Le journal, dès qu'une installation est nommée : les commandes longues
    // — `init`, `creer`, `overframe` — y laissent une trace consultable après
    // coup, et la couche console duplique sur la sortie d'erreur.
    // La mémoire des requêtes — seulement quand la cible est une installation
    // (un dossier qui a son `bdd/`) : plusieurs commandes prennent un fichier.
    if let Some(racine) = cible.as_ref().filter(|r| r.join("bdd").is_dir()) {
        ygo_sources::cache::activer(&Paths::depuis_racine(racine).cache_http());
    }

    let _journal = cible.as_ref().and_then(|racine| {
        let paths = Paths::depuis_racine(racine);
        let garde = ygo_core::log::installer(paths.logs()).ok()?;
        // L'en-tête marque le début du run : sans lui, deux exécutions
        // successives se lisent comme une seule dans le fichier.
        ygo_core::log::entete_session(env!("CARGO_PKG_VERSION"));
        tracing::info!(commande, "ygo-cli");
        Some(garde)
    });

    match commande {
        "schema" => cmd_schema(&cible.context("chemin de l'installation attendu")?),
        "config" => cmd_config(&cible.context("chemin de l'installation attendu")?),
        "creer-vide" => cmd_creer_vide(&cible.context("dossier de destination attendu")?),
        "adopter" => cmd_adopter(reste),
        "init" => cmd_init(&cible.context("chemin de l'installation attendu")?, reste).await,
        "classeur" => {
            let installation = cible.context("chemin de l'installation attendu")?;
            let code = reste.get(1).context("code du set attendu")?;
            cmd_classeur(&installation, code, reste)
        }
        "overframe" => cmd_overframe(&cible.context("chemin de cardinfo.db attendu")?, reste).await,
        "creer" => {
            let installation = cible.context("chemin de l'installation attendu")?;
            let code = reste.get(1).context("code du set attendu")?.clone();
            cmd_creer(&installation, &code, reste).await
        }
        "artworks" => {
            cmd_artworks(&cible.context("chemin du .db du classeur attendu")?, reste).await
        }
        "raretes" => cmd_raretes(&cible.context("chemin de l'installation attendu")?, reste),
        "images-tirage" => {
            cmd_images_tirage(&cible.context("chemin de l'installation attendu")?, reste)
        }
        "noms-fr" => cmd_noms_fr(&cible.context("chemin de l'installation attendu")?, reste),
        "reprise-images" => {
            cmd_reprise_images(&cible.context("chemin de l'installation attendu")?, reste).await
        }
        "numeros-absents" => {
            cmd_numeros_absents(&cible.context("chemin de l'installation attendu")?, reste).await
        }
        "simuler-numeros" => {
            cmd_simuler_numeros(&cible.context("chemin de l'installation attendu")?, reste).await
        }
        "etats-defaut" => {
            cmd_etats_defaut(&cible.context("chemin de l'installation attendu")?, reste)
        }
        "doublons" => cmd_doublons(&cible.context("chemin de l'installation attendu")?, reste),
        "anomalies" => cmd_anomalies(&cible.context("chemin de l'installation attendu")?, reste),
        "images" => cmd_images(&cible.context("chemin de l'installation attendu")?, reste).await,
        "importer" => {
            let installation = cible.context("chemin de l'installation attendu")?;
            let fichier = reste.get(1).context("fichier CSV attendu")?;
            cmd_importer(&installation, Path::new(fichier), reste).await
        }
        "exporter" => {
            let installation = cible.context("chemin de l'installation attendu")?;
            let fichier = reste.get(1).context("fichier CSV de destination attendu")?;
            cmd_exporter(&installation, Path::new(fichier), reste)
        }
        "inventaire" => cmd_inventaire(&cible.context("chemin de l'installation attendu")?, reste),
        "etat" => cmd_etat(&cible.context("chemin de l'installation attendu")?, reste),
        "stats" => cmd_stats(&cible.context("chemin de l'installation attendu")?, reste),
        "comparer" => {
            let reference = cible.context("base de référence attendue")?;
            let candidate = reste
                .get(1)
                .map(PathBuf::from)
                .context("base candidate attendue")?;
            cmd_comparer(&reference, &candidate)
        }
        "--version" | "-V" => {
            println!("ygo-cli {VERSION}");
            Ok(true)
        }
        "--help" | "-h" | "aide" => {
            aide();
            Ok(true)
        }
        autre => bail!("commande inconnue : `{autre}` — voir `ygo-cli --help`"),
    }
}

fn aide() {
    println!(
        "ygo-cli {VERSION} — diagnostic du portage Yu-Gi-Oh! Collection Manager

USAGE
  ygo-cli schema <installation>    vérifie les schémas SQLite d'une installation
  ygo-cli config <installation>    affiche les préférences telles que le Rust les lit
  ygo-cli creer-vide <dossier>     crée une arborescence et des bases vides conformes
  ygo-cli adopter <source> <dest>  reprend une installation dans une racine neuve
  ygo-cli init <installation>      construit bdd/cardinfo.db
        [--archive <aggregate.zip>]   hors ligne, depuis une archive locale
        [--catalogue <cardinfo.json>] avec une réponse YGOPRODeck enregistrée
        [--sur-place]                 sans passer par un fichier temporaire
  ygo-cli classeur <installation> <CODE>
                                   construit les lignes d'un classeur depuis cardinfo.db
        [--ecrire <fichier.db>]       et écrit la base du classeur
  ygo-cli overframe <cardinfo.db>  complète les tirages Overframe depuis Yugipedia
        [--force]                     ignore la garde d'idempotence par révision
        [--simuler]                   annule tout à la fin — n'écrit rien
  ygo-cli creer <installation> <CODE>
                                   CRÉE le classeur d'un set, artworks compris
        [--sans-artworks]             s'arrête après l'écriture, sans réseau Yugipedia
  ygo-cli artworks <classeur.db>   aligne un classeur sur la Set list Yugipedia
        [--langue XX]                 force la langue de la page (défaut : le set_code)
        [--simuler]                   annule tout à la fin — n'écrit rien
        [--detail]                    dit ligne par ligne ce qui change
  ygo-cli raretes <installation>   canonise les libellés de rareté des classeurs
        [--corriger]                  écrit — sans lui, rien n'est modifié
        [--classeur CODE]             se limite à un classeur
  ygo-cli images-tirage <installation>  pose l'image Yugipedia de chaque tirage
        [--corriger]                  écrit — sans lui, rien n'est modifié
        [--classeur CODE]             se limite à un classeur
  ygo-cli noms-fr <installation>   pose les noms FR officiels de la base dans les classeurs
        [--corriger]                  écrit — sans lui, rien n'est modifié
        [--classeur CODE]             se limite à un classeur
  ygo-cli reprise-images <installation>  demande à Yugipedia quels fichiers de repli il a désormais
        [--corriger]                  écrit — sans lui, rien n'est modifié
  ygo-cli numeros-absents <installation>  ajoute les numéros que la Set list Yugipedia
                                   connaît et que la base a perdus (LOCH-JP013…)
        [--corriger]                  écrit — sans lui, rien n'est modifié
        [--classeur CODE]             se limite à un classeur
  ygo-cli simuler-numeros <installation>  crée des classeurs d'essai À PART et y
                                   applique numeros-absents + la passe artworks ;
                                   rapport dans simulation_numeros.md, rien n'est
                                   touché dans vos classeurs
        [--nombre N]                  nombre de sets tirés au sort (défaut 50,
                                      moitié anglais, moitié japonais)
        [--graine N]                  autre tirage au sort
        [--codes A,B,C]               ces sets-là plutôt qu'un tirage
  ygo-cli etats-defaut <installation>  pose Mint et l'édition connue sur les possédées qui n'en ont pas
        [--corriger]                  écrit — sans lui, rien n'est modifié
        [--classeur CODE]             se limite à un classeur
  ygo-cli doublons <installation>  retire les lignes insérées à tort par la passe artworks
        [--corriger]                  supprime — sans lui, rien n'est modifié
        [--classeur CODE]             se limite à un classeur
  ygo-cli anomalies <installation> les artworks qu'un classeur devrait avoir et n'a pas
        [--scanner]                   relit cardinfo.db et met la table à jour
        [--corriger]                  ajoute les lignes manquantes aux classeurs
        [--classeur CODE]             se limite à un classeur
        [--detail]                    liste les anomalies une par une
  ygo-cli images <installation>    télécharge les images manquantes des classeurs
        [--telecharger]               écrit — sans lui, rien n'est téléchargé
        [--classeur CODE]             se limite à un classeur
  ygo-cli importer <installation> <fichier.csv>
                                   importe une collection au format Scanflip
        [--ecrire]                    écrit — sans lui, rien n'est modifié
        [--creer-classeurs]           crée d'abord les classeurs que le CSV
                                      nomme et que l'installation n'a pas
        [--classeur CODE]             se limite aux lignes de ce classeur
        [--detail]                    liste les lignes refusées et les fusions
  ygo-cli exporter <installation> <fichier.csv>
                                   exporte les cartes possédées au format Scanflip
        [--anglais]                   codes et libellé en anglais (défaut : français)
        [--classeur CODE]             un seul classeur
  ygo-cli inventaire <installation>
                                   liste les cartes possédées, tous classeurs confondus
        [--sous-playset]              seulement celles qu'on n'a pas en trois exemplaires
        [--sans-etat]                 seulement celles dont l'état n'est pas renseigné
        [--classeur CODE] [--rarete R] [--nom TEXTE]
  ygo-cli etat <installation> <ÉTAT>
                                   renseigne l'état d'un LOT de cartes (NM, M, EX…)
        [--ecrire]                    écrit — sans lui, rien n'est modifié
        [--sans-etat]                 ne vise que celles qui n'en ont pas (le cas courant)
        [--edition 1st|unlimited|limited]  écrit aussi l'édition
        [--classeur CODE] [--rarete R] [--nom TEXTE]
  ygo-cli stats <installation>     où en est chaque classeur, et sur quelles raretés
        [--raretes]                   la décomposition cumulée de la collection
        [--classeur CODE]             se limite à un classeur
  ygo-cli comparer <référence> <candidate>
                                   compare deux cardinfo.db table par table
  ygo-cli --version

<installation> est le dossier contenant bdd/, img/ et logs/.
Exemples :
  ygo-cli schema \"Projet Python/V1.0.3\"
  ygo-cli init ./essai --archive ./aggregate.zip
  ygo-cli classeur \"Projet Python/V1.0.3\" RA02
  ygo-cli creer ./essai RA02
  ygo-cli overframe ./essai/bdd/cardinfo.db
  ygo-cli artworks \"Projet Python/V1.0.3/bdd/classeur_creer/LOCR-JP/LOCR-JP.db\" --simuler
  ygo-cli comparer \"Projet Python/V1.0.4/bdd/cardinfo.db\" ./essai/bdd/cardinfo.db

`overframe` et `artworks` MODIFIENT la base indiquée : travaillez sur une
copie, ou passez --simuler pour voir ce que la passe changerait sans rien
écrire. `creer` ÉCRIT un nouveau classeur dans l'installation ; il ne touche
jamais un classeur déjà peuplé."
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// schema
// ─────────────────────────────────────────────────────────────────────────────

/// Vérifie `cardinfo.db` et chaque classeur d'une installation contre les DDL
/// de référence.
///
/// C'est le premier test de non-régression du portage, et il s'exécute sur les
/// données réelles de l'utilisateur — en **lecture seule**.
fn cmd_schema(installation: &Path) -> anyhow::Result<bool> {
    let paths = Paths::depuis_racine(installation);
    let mut tout_va_bien = true;

    println!("Installation : {}", installation.display());
    println!();

    // cardinfo.db
    let cardinfo = paths.cardinfo_db();
    if cardinfo.is_file() {
        let divergences = schema::verifier_fichier(&cardinfo, DDL_CARDINFO)
            .with_context(|| format!("lecture de {}", cardinfo.display()))?;
        tout_va_bien &= rapporter("cardinfo.db", &divergences);
        afficher_comptages(&cardinfo)?;
    } else {
        println!("  cardinfo.db     ABSENTE");
    }
    println!();

    // Classeurs
    let classeurs = paths.classeurs_existants();
    println!("Classeurs : {}", classeurs.len());
    let mut total_cartes = 0i64;
    let mut total_possedees = 0i64;

    for code in &classeurs {
        let base = paths.classeur_db(code);
        let divergences = schema::verifier_fichier(&base, DDL_CLASSEUR)
            .with_context(|| format!("lecture de {}", base.display()))?;

        let conn = connexion::ouvrir_lecture_seule(&base)?;
        let cartes: i64 = conn.query_row("SELECT count(*) FROM cards", [], |r| r.get(0))?;
        let possedees: i64 =
            conn.query_row("SELECT count(*) FROM cards WHERE possessed = 1", [], |r| {
                r.get(0)
            })?;
        let colonnes = migrations::colonnes(&conn, "cards")?.len();
        total_cartes += cartes;
        total_possedees += possedees;

        // Un classeur n'a pas de table hors périmètre : toute divergence en
        // est une.
        let etat = if divergences.is_empty() {
            "OK  "
        } else {
            "ÉCART"
        };
        println!(
            "  {etat} {code:<10} {cartes:>5} cartes  {possedees:>4} possédées  {colonnes} colonnes"
        );
        for d in &divergences {
            println!("        → {d}");
            tout_va_bien = false;
        }
    }

    println!();
    println!("  Total : {total_cartes} cartes, {total_possedees} possédées");
    println!();
    println!(
        "{}",
        if tout_va_bien {
            "Conclusion : tous les schémas sont conformes aux DDL de référence."
        } else {
            "Conclusion : au moins un schéma diverge — voir les écarts ci-dessus."
        }
    );
    Ok(tout_va_bien)
}

/// Index appartenant à une table hors périmètre d'initialisation.
const INDEX_HORS_INIT: [&str; 1] = ["idx_cie_name_set"];

/// Un objet manquant qui n'appartient pas au schéma d'initialisation.
///
/// `anomalies`, `overframe_sync` et `card_images_externes` sont créées à la
/// demande par les fonctionnalités qui les possèdent. Une installation où la
/// détection d'anomalies ou la passe Overframe n'a jamais tourné ne les a
/// simplement pas — ce n'est pas une divergence de schéma. C'est le cas de la
/// V1.0.3, qui porte 10 tables là où la V1.0.4 en a 11.
fn est_hors_init(d: &schema::Divergence) -> bool {
    use ygo_db::init::TABLES_HORS_INIT;
    match d {
        schema::Divergence::Manquant { genre, nom } => match genre.as_str() {
            "table" => TABLES_HORS_INIT.contains(&nom.as_str()),
            "index" => INDEX_HORS_INIT.contains(&nom.as_str()),
            _ => false,
        },
        _ => false,
    }
}

fn rapporter(nom: &str, divergences: &[schema::Divergence]) -> bool {
    let (hors_init, reelles): (Vec<_>, Vec<_>) = divergences.iter().partition(|d| est_hors_init(d));

    if reelles.is_empty() {
        println!("  OK   {nom}");
    } else {
        println!("  ÉCART {nom} — {} divergence(s)", reelles.len());
        for d in &reelles {
            println!("        → {d}");
        }
    }
    if !hors_init.is_empty() {
        let noms: Vec<String> = hors_init
            .iter()
            .map(|d| match d {
                schema::Divergence::Manquant { nom, .. } => nom.clone(),
                autre => autre.to_string(),
            })
            .collect();
        println!(
            "        (absentes, hors périmètre d'initialisation : {})",
            noms.join(", ")
        );
    }
    reelles.is_empty()
}

fn afficher_comptages(cardinfo: &Path) -> anyhow::Result<()> {
    let conn = connexion::ouvrir_lecture_seule(cardinfo)?;
    let mut requete = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )?;
    let tables: Vec<String> = requete
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<_, _>>()?;

    for table in tables {
        let n: i64 = conn.query_row(&format!("SELECT count(*) FROM \"{table}\""), [], |r| {
            r.get(0)
        })?;
        println!("        {table:<22} {n:>7}");
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// config
// ─────────────────────────────────────────────────────────────────────────────

/// Affiche les préférences telles que le Rust les interprète.
///
/// Sert à vérifier d'un coup d'œil que les garde-fous du §3.5 se comportent
/// comme en Python sur un fichier réel — y compris le `app_config.json` de la
/// V1.0.3, qui ne contient qu'une seule clé.
fn cmd_config(installation: &Path) -> anyhow::Result<bool> {
    let paths = Paths::depuis_racine(installation);
    let cfg = Config::charger(paths.app_config());

    println!("Installation : {}", installation.display());
    println!();
    println!("app_config.json  ({})", etat_fichier(&paths.app_config()));
    println!("  langue                     {}", cfg.langue().code());
    println!("  ui_langue                  {}", cfg.ui_langue().code());
    println!("  image_source               {}", cfg.source_image().code());
    println!("  font_scale                 {}", cfg.font_scale());
    let (cols, lignes) = cfg.grille_defaut();
    println!("  grille_defaut              {cols}×{lignes}");
    let ordre: Vec<&str> = cfg.ordre_tri().iter().map(|c| c.code()).collect();
    println!("  ordre_tri_criteres         {}", ordre.join(" → "));
    println!(
        "  n_raretes_par_artwork      {}",
        cfg.n_raretes_par_artwork()
    );
    println!(
        "  completer_artworks_yugi.   {}",
        cfg.completer_artworks_yugipedia()
    );
    println!(
        "  langues_locales_actives    {:?}",
        cfg.langues_locales_actives()
    );

    println!();
    let priorites = Priorites::charger(paths.rarity_config());
    println!(
        "rarity_config.json  ({})",
        etat_fichier(&paths.rarity_config())
    );
    println!("  {} rareté(s) déclarée(s)", priorites.len());
    if priorites.is_empty() {
        // `etat_fichier` dit « absent — valeurs par défaut », ce qui est vrai
        // d'`app_config.json` et faux d'ici : une table absente n'a AUCUN
        // défaut, toutes les raretés tombent sur leur valeur d'inconnue. Le
        // fichier s'écrit à la première initialisation ou à l'ouverture des
        // Options ; `config` ne le crée pas — une commande de diagnostic
        // n'écrit rien.
        println!("  ⚠ aucune priorité : le tri par rareté ne range rien.");
        println!("     Le fichier s'écrit à l'initialisation de la base.");
    }

    println!();
    let infos = version::charger(paths.last_update());
    println!("last_update.txt  ({})", etat_fichier(&paths.last_update()));
    match version::version_locale(infos.as_deref()) {
        Some(v) => println!("  database_version           {v}"),
        None => println!("  aucune version connue"),
    }

    Ok(true)
}

fn etat_fichier(chemin: &Path) -> &'static str {
    if chemin.is_file() {
        "présent"
    } else {
        "absent — valeurs par défaut"
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// creer-vide
// ─────────────────────────────────────────────────────────────────────────────

/// Crée une arborescence complète avec une `cardinfo.db` vide mais conforme.
///
/// Utile pour partir d'une installation propre sans lancer le Python.
fn cmd_creer_vide(dossier: &Path) -> anyhow::Result<bool> {
    let paths = Paths::depuis_racine(dossier);
    paths.creer_dossiers()?;

    let bases = connexion::Bases::new();
    let cardinfo = paths.cardinfo_db();
    if cardinfo.exists() {
        bail!("{} existe déjà — refus d'écraser", cardinfo.display());
    }

    bases.avec_base(&cardinfo, |tx| {
        schema::creer_tables(tx, DDL_CARDINFO)?;
        schema::creer_index(tx, DDL_CARDINFO)?;
        Ok(())
    })?;

    let divergences = schema::verifier_fichier(&cardinfo, DDL_CARDINFO)?;
    println!("Arborescence créée sous {}", dossier.display());
    println!("  bdd/cardinfo.db  11 tables, 10 index");
    Ok(rapporter("vérification du schéma", &divergences))
}

// ─────────────────────────────────────────────────────────────────────────────
// adopter
// ─────────────────────────────────────────────────────────────────────────────

/// `adopter` — reprend une installation existante dans une racine neuve.
///
/// Annonce d'abord ce qui sera copié et combien cela pèse : sur un disque
/// presque plein, engager 300 Mo sans le dire serait discourtois. Puis copie,
/// puis rappelle ce qui reste à faire — la base, qui ne se copie pas.
fn cmd_adopter(args: &[String]) -> anyhow::Result<bool> {
    let (source, destination) = ygo_app::adoption::arguments(args)?;

    println!("Source      : {}", source.display());
    println!("Destination : {}", destination.display());
    println!();

    let mesures = ygo_app::adoption::a_reprendre(&source);
    if mesures.is_empty() {
        bail!(
            "{} ne contient rien à reprendre — est-ce bien une installation ?",
            source.display()
        );
    }
    println!("À reprendre :");
    let mut total = 0_u64;
    for (nom, octets) in &mesures {
        total += octets;
        println!("  {nom:<12} {:>6} Mo", octets / 1_048_576);
    }
    println!("  {:<12} {:>6} Mo", "total", total / 1_048_576);
    println!();
    println!("Non repris :");
    for (quoi, raison) in ygo_app::adoption::non_repris() {
        println!("  {quoi:<26} {raison}");
    }
    println!();

    let bilan = ygo_app::adoption::adopter(&source, &destination)?;

    println!("Repris :");
    println!(
        "  classeurs     {:>6}   {}",
        bilan.classeurs.len(),
        bilan.classeurs.join(", ")
    );
    if bilan.rebuts > 0 {
        println!("  corbeille     {:>6}", bilan.rebuts);
    }
    println!("  images        {:>6}", bilan.images);
    if bilan.exports > 0 {
        println!("  export        {:>6}", bilan.exports);
    }
    println!(
        "  réglages      {:>6}   {}",
        bilan.reglages.len(),
        bilan.reglages.join(", ")
    );
    println!("  copié         {:>6} Mo", bilan.mo());
    println!();
    println!("La source n'a pas été touchée.");
    println!(
        "Reste à construire la base : ygo-cli init \"{}\"",
        destination.display()
    );
    Ok(true)
}

// ─────────────────────────────────────────────────────────────────────────────
// init
// ─────────────────────────────────────────────────────────────────────────────

/// Construit `bdd/cardinfo.db`.
///
/// Sans option, tout vient du réseau. Avec `--archive`, l'initialisation est
/// **hors ligne** : c'est le mode qui permet de rejouer le pipeline sur des
/// fixtures figées, donc de comparer le résultat à celui de la V1.0.4 sans
/// dépendre d'un wiki vivant.
async fn cmd_init(installation: &Path, args: &[String]) -> anyhow::Result<bool> {
    let paths = Paths::depuis_racine(installation);

    let mut options = Options::default();
    let mut i = 1;
    while i < args.len() {
        match args.get(i).map(String::as_str) {
            Some("--archive") => {
                let chemin = args.get(i + 1).context("--archive attend un chemin")?;
                options.source = Source::Locale {
                    archive: PathBuf::from(chemin),
                    catalogue: None,
                };
                i += 2;
            }
            Some("--catalogue") => {
                let chemin = args.get(i + 1).context("--catalogue attend un chemin")?;
                match &mut options.source {
                    Source::Locale { catalogue, .. } => *catalogue = Some(PathBuf::from(chemin)),
                    Source::Reseau => bail!("--catalogue suppose --archive"),
                }
                i += 2;
            }
            Some("--sur-place") => {
                options.via_fichier_temporaire = false;
                i += 1;
            }
            Some(autre) => bail!("option inconnue : {autre}"),
            None => break,
        }
    }

    println!("Installation : {}", installation.display());
    match &options.source {
        Source::Reseau => println!("Source       : réseau (GitHub + YGOPRODeck)"),
        Source::Locale { archive, catalogue } => {
            println!("Source       : hors ligne");
            println!("  archive    : {}", archive.display());
            println!(
                "  catalogue  : {}",
                catalogue.as_ref().map_or_else(
                    || "aucun (pas de statistiques)".to_owned(),
                    |c| c.display().to_string()
                )
            );
        }
    }
    println!();

    // La progression passe par un canal ; le binaire l'affiche, l'interface
    // fera autrement. Le crate métier ne sait pas laquelle des deux écoute.
    let (envoi, mut reception) = tokio::sync::mpsc::unbounded_channel();
    let affichage = tokio::spawn(async move {
        // Une ligne tous les 10 % : l'archive arrive en des centaines de blocs,
        // et un journal de cent lignes de progression n'apprend rien de plus
        // qu'un journal de dix.
        const PAS_POURCENT: u64 = 10;
        let mut dernier_palier = u64::MAX;
        while let Some(etape) = reception.recv().await {
            match etape {
                Etape::TelechargementArchive { recus, total } => {
                    if let Some(t) = total.filter(|t| *t > 0) {
                        let palier = (recus * 100 / t) / PAS_POURCENT;
                        if palier != dernier_palier {
                            dernier_palier = palier;
                            println!(
                                "  téléchargement {:>3} %  ({} / {} Mo)",
                                palier * PAS_POURCENT,
                                recus / 1_048_576,
                                t / 1_048_576
                            );
                        }
                    }
                }
                Etape::ArchiveRecue { octets } => {
                    println!("  archive           {} Mo sur disque", octets / 1_048_576);
                }
                Etape::CatalogueRecu { entrees } => {
                    println!("  catalogue         {entrees} entrées");
                }
                Etape::CatalogueIndisponible => {
                    println!("  catalogue         indisponible — pas de statistiques");
                }
                Etape::CartesParsees {
                    cartes,
                    textes,
                    images,
                } => {
                    println!("  cartes            {cartes}, {textes} textes, {images} images");
                }
                Etape::SetsParses {
                    sets,
                    locales,
                    tirages,
                } => {
                    println!("  sets              {sets}, {locales} locales, {tirages} tirages");
                }
                Etape::ArtworksResolus { ajoutes } => {
                    println!("  artworks          +{ajoutes} tirage(s) par expansion");
                }
                Etape::CartesEnrichies { enrichies } => {
                    println!("  enrichissement    {enrichies} carte(s)");
                }
                Etape::EcritureBase => println!("  écriture en base…"),
                Etape::Terminee(_) => {}
            }
        }
    });

    // Par le réseau, l'initialisation passe par `maj` : c'est elle qui écrit
    // `bdd/last_update.txt`, sans quoi la base se déclarerait périmée aussitôt
    // construite. Hors ligne, il n'y a pas de version à demander.
    let (comptages, version) = match options.source {
        Source::Reseau => {
            let client = ygo_sources::ClientHttp::new()?;
            let bilan =
                ygo_app::maj::mettre_a_jour(&paths, &client, &options, Some(&envoi)).await?;
            (bilan.comptages, bilan.version)
        }
        Source::Locale { .. } => (
            ygo_app::init::initialiser(&paths, &options, Some(&envoi)).await?,
            None,
        ),
    };
    drop(envoi);
    let _ = affichage.await;

    println!();
    println!("Base construite : {}", paths.cardinfo_db().display());
    println!("  sets                   {:>7}", comptages.sets);
    println!("  set_locales            {:>7}", comptages.locales);
    println!("  cards                  {:>7}", comptages.cartes);
    println!("  card_texts             {:>7}", comptages.textes);
    println!("  card_images            {:>7}", comptages.images);
    println!("  set_prints             {:>7}", comptages.tirages);
    println!(
        "  cards_missing_fr       {:>7}",
        comptages.sans_traduction_fr
    );
    if comptages.tirages_orphelins > 0 {
        println!(
            "  tirages écartés        {:>7}  (locale non déclarée par le set)",
            comptages.tirages_orphelins
        );
    }
    match version {
        Some(v) => println!("  version enregistrée    {v:>7}"),
        None => println!("  version                    non enregistrée"),
    }
    Ok(true)
}

// ─────────────────────────────────────────────────────────────────────────────
// comparer
// ─────────────────────────────────────────────────────────────────────────────

/// Compare deux `cardinfo.db` table par table.
///
/// C'est le test qui compte : la base produite par le Rust face à celle que la
/// V1.0.4 Python a réellement écrite. Les deux fichiers sont ouverts en
/// **lecture seule** — la base de référence ne doit courir aucun risque.
/// `raretes` — canonise les libellés de rareté des classeurs d'une installation.
///
/// Sans `--corriger`, la commande **n'écrit rien** : elle dit ce qu'elle
/// ferait. C'est la forme par défaut à dessein — une passe qui réécrit 912
/// lignes réparties sur 26 bases mérite d'être lue avant d'être lancée.
fn cmd_raretes(installation: &Path, args: &[String]) -> anyhow::Result<bool> {
    let corriger = args.iter().any(|a| a == "--corriger");
    let seul = args
        .iter()
        .position(|a| a == "--classeur")
        .and_then(|i| args.get(i + 1))
        .map(|c| c.to_uppercase());

    let paths = Paths::depuis_racine(installation);
    let reference = Priorites::charger(paths.rarity_config());
    println!("Installation : {}", installation.display());
    println!(
        "Référence    : {} raretés dans rarity_config.json",
        reference.len()
    );
    if reference.is_empty() {
        println!("  ATTENTION : liste des Options vide ou illisible — seules les");
        println!("  abréviations et les alias du référentiel pourront être résolus.");
    }
    println!(
        "Mode         : {}",
        if corriger {
            "ÉCRITURE"
        } else {
            "analyse seule — rien ne sera modifié"
        }
    );
    println!();

    let classeurs = accueil::lister(&paths, (3, 3));
    let mut total_corrections = 0_usize;
    let mut total_lignes = 0_usize;
    let mut inconnues_globales: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();

    for classeur in &classeurs {
        if seul.as_deref().is_some_and(|c| c != classeur.code) {
            continue;
        }
        let db = paths.classeur_db(&classeur.code);
        if !db.is_file() {
            continue;
        }
        let mut conn = rusqlite::Connection::open(&db)?;
        let rapport = raretes::analyser(&conn, &reference)?;
        total_lignes += rapport.lues;
        for (libelle, n) in &rapport.inconnues {
            *inconnues_globales.entry(libelle.clone()).or_insert(0) += n;
        }
        if rapport.vide() && rapport.inconnues.is_empty() {
            continue;
        }

        println!("{} — {} ligne(s)", classeur.code, rapport.lues);
        for ((avant, apres), n) in rapport.par_cible() {
            println!("  {n:5}  {avant}  →  {apres}");
        }
        for (libelle, n) in &rapport.inconnues {
            println!("  {n:5}  {libelle}  →  (non reconnu, laissé tel quel)");
        }
        total_corrections += rapport.corrections.len();

        if corriger && !rapport.vide() {
            let ecrites = raretes::appliquer(&mut conn, &rapport)?;
            println!("  {ecrites} ligne(s) réécrite(s)");
        }
        println!();
    }

    println!(
        "{} classeur(s), {total_lignes} ligne(s), {total_corrections} à canoniser",
        classeurs.len()
    );
    if !inconnues_globales.is_empty() {
        println!();
        println!("Libellés qu'aucune table ne reconnaît — à examiner, jamais devinés :");
        for (libelle, n) in &inconnues_globales {
            println!("  {n:5}  {libelle}");
        }
    }
    if !corriger && total_corrections > 0 {
        println!();
        println!("Relancer avec --corriger pour écrire.");
    }
    Ok(true)
}

/// `noms-fr` — ce que chaque mise à jour de la base fait d'elle-même pour les
/// noms français ([`ygo_app::noms_fr`]) : poser le nom officiel là où il
/// manque, remplacer celui qui en diffère, ne rien inventer, ne rien effacer.
/// Sans `--corriger`, elle **n'écrit rien**.
fn cmd_noms_fr(installation: &Path, args: &[String]) -> anyhow::Result<bool> {
    let corriger = args.iter().any(|a| a == "--corriger");
    let seul = args
        .iter()
        .position(|a| a == "--classeur")
        .and_then(|i| args.get(i + 1))
        .map(|c| c.to_uppercase());
    let paths = Paths::depuis_racine(installation);
    let cardinfo = ygo_db::connexion::ouvrir_lecture_seule(paths.cardinfo_db())
        .context("cardinfo.db illisible")?;
    let index = ygo_app::noms_fr::Index::charger(&cardinfo)?;
    println!("Installation : {}", installation.display());
    println!(
        "Mode         : {}",
        if corriger {
            "ÉCRITURE"
        } else {
            "analyse seule — rien ne sera modifié"
        }
    );
    println!();
    println!(
        "{:10} {:>6} {:>8} {:>8} {:>11} {:>12}",
        "classeur", "lignes", "ajoutés", "corrigés", "sans source", "introuvables"
    );
    let (mut ajoutes, mut corriges, mut sans_source, mut introuvables) = (0, 0, 0, 0);
    for code in paths.classeurs_existants() {
        if seul.as_deref().is_some_and(|c| c != code) {
            continue;
        }
        let mut conn = rusqlite::Connection::open(paths.classeur_db(&code))?;
        let r = ygo_app::noms_fr::analyser(&conn, &index)?;
        println!(
            "{:10} {:>6} {:>8} {:>8} {:>11} {:>12}",
            code, r.lues, r.ajoutes, r.corriges, r.sans_source, r.introuvables
        );
        if corriger && !r.a_poser.is_empty() {
            ygo_app::noms_fr::appliquer(&mut conn, &r)?;
        }
        ajoutes += r.ajoutes;
        corriges += r.corriges;
        sans_source += r.sans_source;
        introuvables += r.introuvables;
    }
    println!();
    println!(
        "{ajoutes} nom(s) FR posé(s), {corriges} corrigé(s) ; {sans_source} ligne(s) restent sans \
         nom FR faute de source, {introuvables} carte(s) non retrouvée(s)."
    );
    if corriger {
        println!("Écrit.");
    } else if ajoutes + corriges > 0 {
        println!("Relancer avec --corriger pour écrire.");
    }
    Ok(true)
}

/// `reprise-images` — ce que chaque mise à jour de la base fait d'elle-même
/// ([`ygo_app::replis::reprendre`]) : reposer les images de tirage depuis la
/// base, puis effacer les images servies par la source de repli et oublier
/// leur 404, pour que la prochaine ouverture du classeur retente Yugipedia.
///
/// Elle demande à Yugipedia, une requête pour 50 fichiers, lesquels il a
/// désormais. Sans `--corriger`, elle **n'écrit rien**.
/// `numeros-absents` — les numéros de la Set list que le classeur ignore,
/// retrouvés dans la base et ajoutés (cf. `ygo_app::numeros_absents`).
///
/// Avec `--corriger`, chaque classeur qui reçoit des numéros repasse aussitôt
/// par la passe artworks, pour que ses nouvelles lignes aient l'image de leur
/// tirage. Les images elles-mêmes arrivent à l'ouverture du classeur.
async fn cmd_numeros_absents(installation: &Path, args: &[String]) -> anyhow::Result<bool> {
    let corriger = args.iter().any(|a| a == "--corriger");
    let seul = args
        .iter()
        .position(|a| a == "--classeur")
        .and_then(|i| args.get(i + 1))
        .map(|c| c.to_uppercase());
    let paths = Paths::depuis_racine(installation);
    ygo_sources::cache::activer(&paths.cache_http());
    let raretes = Priorites::charger(paths.rarity_config());
    let client = ygo_sources::ClientHttp::new()?;
    println!("Installation : {}", installation.display());
    println!(
        "Mode         : {}",
        if corriger {
            "ÉCRITURE"
        } else {
            "analyse seule — rien ne sera modifié"
        }
    );
    println!();
    let (mut numeros, mut lignes, mut introuvables) = (0usize, 0usize, 0usize);
    for code in paths.classeurs_existants() {
        if seul.as_deref().is_some_and(|c| c != code) {
            continue;
        }
        let issue =
            match ygo_app::numeros_absents::completer(&paths, &client, &code, &raretes, corriger)
                .await
            {
                Ok(i) => i,
                Err(e) => {
                    println!("{code:10} erreur : {e}");
                    continue;
                }
            };
        let bilan = match issue {
            ygo_app::numeros_absents::Issue::Fait(b) => b,
            ygo_app::numeros_absents::Issue::ClasseurVide => {
                println!("{code:10} classeur vide");
                continue;
            }
            ygo_app::numeros_absents::Issue::PageIntrouvable => {
                println!("{code:10} aucune Set Card List sur Yugipedia");
                continue;
            }
        };
        if bilan.absents == 0 && bilan.images == 0 {
            println!("{code:10} complet");
            continue;
        }
        println!(
            "{code:10} {} numéro(s) absent(s) : {} retrouvé(s), {} introuvable(s)",
            bilan.absents,
            bilan.ajouts.len(),
            bilan.introuvables.len()
        );
        for a in &bilan.ajouts {
            println!("    + {} {} ({} ligne(s))", a.numero, a.nom, a.lignes);
        }
        for (numero, nom) in &bilan.introuvables {
            println!("    ? {numero} « {nom} » — absente de la base, rien n'est créé");
        }
        numeros += bilan.ajouts.len();
        lignes += bilan.lignes();
        introuvables += bilan.introuvables.len();
        if bilan.images > 0 {
            println!(
                "    {} ligne(s) reçoivent l'image Yugipedia de leur tirage",
                bilan.images
            );
        }
        // Les tirages en variante des numéros ajoutés — maintenant ou lors
        // d'un passage précédent — passent par la passe artworks.
        if corriger && (!bilan.ajouts.is_empty() || bilan.images > 0) {
            match ygo_app::creation::completer_artworks(&paths, &client, &code, &raretes).await {
                Ok(_) => println!("    passe artworks refaite"),
                Err(e) => println!("    passe artworks : {e}"),
            }
        }
    }
    println!();
    println!(
        "{numeros} numéro(s) {} ({lignes} ligne(s)), {introuvables} introuvable(s).",
        if corriger { "ajouté(s)" } else { "à ajouter" }
    );
    if !corriger && numeros > 0 {
        println!("Relancer avec --corriger pour écrire.");
    }
    Ok(true)
}

/// Les sets à simuler : ceux demandés, ou un tirage reproductible.
///
/// Le tirage prend des sets d'au moins vingt tirages, moitié anglais
/// (`RA04`), moitié japonais (`LOCH-JP`), en écartant ceux que l'installation
/// a déjà. Un générateur congruentiel suffit : il ne sert qu'à mélanger, et
/// la même graine redonne le même échantillon.
fn sets_a_simuler(
    cardinfo: &rusqlite::Connection,
    deja: &BTreeSet<String>,
    nombre: usize,
    graine: u64,
) -> anyhow::Result<Vec<String>> {
    let mut par_langue: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut requete = cardinfo.prepare(
        "SELECT sl.prefix, sl.language FROM set_locales sl \
           JOIN set_prints sp ON sp.set_locale_id = sl.id \
          WHERE sl.language IN ('en', 'jp') AND sl.prefix LIKE '%-%' \
          GROUP BY sl.id HAVING COUNT(sp.id) >= 20",
    )?;
    let lignes = requete.query_map([], |l| Ok((l.get::<_, String>(0)?, l.get::<_, String>(1)?)))?;
    for ligne in lignes {
        let (prefixe, langue) = ligne?;
        let code = if langue == "jp" {
            prefixe.to_uppercase()
        } else {
            prefixe.split('-').next().unwrap_or_default().to_uppercase()
        };
        // Un préfixe sans code de set (`302-`) ne nomme aucun classeur.
        let valide = !code.is_empty() && !code.ends_with('-') && !code.starts_with('-');
        if valide && !deja.contains(&code) {
            par_langue.entry(langue).or_default().insert(code);
        }
    }
    let mut etat = graine
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1);
    let mut tirer = |codes: &BTreeSet<String>, n: usize| -> Vec<String> {
        let mut v: Vec<String> = codes.iter().cloned().collect();
        for i in (1..v.len()).rev() {
            etat = etat
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let j = usize::try_from((etat >> 33) % (i as u64 + 1)).unwrap_or(0);
            v.swap(i, j);
        }
        v.truncate(n);
        v
    };
    let vide = BTreeSet::new();
    let moitie = nombre / 2;
    let mut choisis = tirer(par_langue.get("en").unwrap_or(&vide), nombre - moitie);
    choisis.extend(tirer(par_langue.get("jp").unwrap_or(&vide), moitie));
    Ok(choisis)
}

/// Un message d'erreur ramené à une cellule de tableau lisible.
fn court(message: &str) -> String {
    let une_ligne = message.replace(['|', '\n'], " ");
    if une_ligne.chars().count() > 100 {
        format!("{}…", une_ligne.chars().take(100).collect::<String>())
    } else {
        une_ligne
    }
}

/// `simuler-numeros` — la complétion des numéros absents, éprouvée sur des
/// classeurs créés pour l'occasion, **à part** de ceux de l'utilisateur.
///
/// Chaque set est créé comme l'interface le crée (lignes locales, ou
/// YGOPRODeck à défaut), puis passe par `numeros_absents` et la passe
/// artworks. Le rapport, `simulation_numeros.md` à la racine de
/// l'installation, liste chaque ajout avec son mode de rapprochement — c'est
/// ce qu'on relit pour juger le garde-fou — et chaque numéro laissé de côté.
///
/// Les requêtes passent par le même client que l'application : quotas et
/// cache sont ceux de l'installation, une seconde par requête Yugipedia.
async fn cmd_simuler_numeros(installation: &Path, args: &[String]) -> anyhow::Result<bool> {
    use std::fmt::Write as _;
    let valeur = |cle: &str| {
        args.iter()
            .position(|a| a == cle)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let nombre: usize = valeur("--nombre")
        .and_then(|n| n.parse().ok())
        .unwrap_or(50);
    let graine: u64 = valeur("--graine")
        .and_then(|n| n.parse().ok())
        .unwrap_or(20_261_010);

    let reelle = Paths::depuis_racine(installation);
    ygo_sources::cache::activer(&reelle.cache_http());
    let raretes = Priorites::charger(reelle.rarity_config());

    // L'installation d'essai : une copie de la base, aucun classeur réel.
    let racine_essai = installation.join("simulation_numeros_tmp");
    if racine_essai.exists() {
        std::fs::remove_dir_all(&racine_essai)?;
    }
    let essai = Paths::depuis_racine(&racine_essai);
    std::fs::create_dir_all(essai.bdd())?;
    std::fs::copy(reelle.cardinfo_db(), essai.cardinfo_db()).context("copie de cardinfo.db")?;
    if reelle.rarity_config().is_file() {
        std::fs::copy(reelle.rarity_config(), essai.rarity_config())?;
    }

    let codes: Vec<String> = match valeur("--codes") {
        Some(liste) => liste
            .split(',')
            .map(|c| c.trim().to_uppercase())
            .filter(|c| !c.is_empty())
            .collect(),
        None => {
            let deja: BTreeSet<String> = reelle.classeurs_existants().into_iter().collect();
            let cardinfo = connexion::ouvrir_lecture_seule(essai.cardinfo_db())?;
            sets_a_simuler(&cardinfo, &deja, nombre, graine)?
        }
    };
    println!("Installation d'essai : {}", racine_essai.display());
    println!(
        "{} set(s) — environ 5 requêtes Yugipedia chacun, une par seconde : ~{} min",
        codes.len(),
        (codes.len() * 6).div_ceil(60)
    );
    println!();

    let client = ygo_sources::ClientHttp::new()?;
    let mut rapport = String::new();
    let mut detail = String::new();
    let _ = writeln!(rapport, "# Simulation — numéros absents\n");
    let _ = writeln!(
        rapport,
        "| Set | Lignes créées | Absents | Ajoutés | Introuvables | Images posées | Passe : illustrations | Remarque |"
    );
    let _ = writeln!(rapport, "|---|---|---|---|---|---|---|---|");
    let (mut t_ajouts, mut t_introuvables, mut t_password, mut t_fiche) =
        (0usize, 0usize, 0usize, 0usize);

    for (i, code) in codes.iter().enumerate() {
        print!("[{}/{}] {code:10} ", i + 1, codes.len());
        let creation =
            creation::creer(&essai, &client, code, false, &creation::Greffons::default()).await;
        let lignes = match creation {
            Ok(creation::Issue::Cree { lignes, .. }) => lignes,
            Ok(creation::Issue::DejaExistant) => {
                println!("déjà là");
                continue;
            }
            Err(e) => {
                println!("création impossible : {e}");
                let _ = writeln!(
                    rapport,
                    "| {code} | — | | | | | | création impossible : {} |",
                    court(&e.to_string())
                );
                continue;
            }
        };
        let issue =
            ygo_app::numeros_absents::completer(&essai, &client, code, &raretes, true).await;
        let bilan = match issue {
            Ok(ygo_app::numeros_absents::Issue::Fait(b)) => b,
            Ok(ygo_app::numeros_absents::Issue::PageIntrouvable) => {
                println!("{lignes} lignes, pas de Set Card List");
                let _ = writeln!(
                    rapport,
                    "| {code} | {lignes} | | | | | | pas de Set Card List |"
                );
                continue;
            }
            Ok(ygo_app::numeros_absents::Issue::ClasseurVide) => {
                println!("classeur vide");
                continue;
            }
            Err(e) => {
                println!("erreur : {e}");
                let _ = writeln!(
                    rapport,
                    "| {code} | {lignes} | | | | | | erreur : {} |",
                    court(&e.to_string())
                );
                continue;
            }
        };
        let passe = creation::completer_artworks(&essai, &client, code, &raretes).await;
        let (illustrations, remarque) = match &passe {
            Ok(a) => (a.bilan.illustrations.to_string(), String::new()),
            Err(e) => (
                String::from("—"),
                format!("passe : {}", court(&e.to_string())),
            ),
        };
        println!(
            "{lignes} lignes, {} absent(s), {} ajouté(s), {} introuvable(s), {} image(s)",
            bilan.absents,
            bilan.ajouts.len(),
            bilan.introuvables.len(),
            bilan.images
        );
        let _ = writeln!(
            rapport,
            "| {code} | {lignes} | {} | {} | {} | {} | {illustrations} | {remarque} |",
            bilan.absents,
            bilan.ajouts.len(),
            bilan.introuvables.len(),
            bilan.images
        );
        t_ajouts += bilan.ajouts.len();
        t_introuvables += bilan.introuvables.len();
        if bilan.ajouts.is_empty() && bilan.introuvables.is_empty() {
            continue;
        }
        let _ = writeln!(detail, "\n### {code}\n");
        for a in &bilan.ajouts {
            match a.via {
                ygo_app::numeros_absents::Rapprochement::Password => t_password += 1,
                ygo_app::numeros_absents::Rapprochement::Fiche => t_fiche += 1,
                ygo_app::numeros_absents::Rapprochement::Nom => {}
            }
            let _ = writeln!(
                detail,
                "- + `{}` **{}** — Set list : « {} » — via {} — {} ligne(s)",
                a.numero,
                a.nom,
                a.nom_set_list,
                a.via.libelle(),
                a.lignes
            );
        }
        for (numero, nom) in &bilan.introuvables {
            let _ = writeln!(detail, "- ? `{numero}` « {nom} » — introuvable, rien créé");
        }
    }

    let _ = writeln!(
        rapport,
        "\n**Total** : {t_ajouts} numéro(s) ajouté(s) — dont {t_fiche} par la fiche, \
         {t_password} par le password — et {t_introuvables} introuvable(s).\n\
         \nÀ relire en priorité : les ajouts « via password » (le nom diffère de la base).\n"
    );
    let _ = writeln!(rapport, "## Détail{detail}");
    let chemin = installation.join("simulation_numeros.md");
    std::fs::write(&chemin, &rapport)?;
    if let Err(e) = std::fs::remove_dir_all(&racine_essai) {
        println!("(installation d'essai non supprimée : {e})");
    }
    println!();
    println!(
        "{t_ajouts} ajouté(s) ({t_fiche} par la fiche, {t_password} par le password), \
         {t_introuvables} introuvable(s)."
    );
    println!("Rapport : {}", chemin.display());
    Ok(true)
}

async fn cmd_reprise_images(installation: &Path, args: &[String]) -> anyhow::Result<bool> {
    let corriger = args.iter().any(|a| a == "--corriger");
    let paths = Paths::depuis_racine(installation);
    ygo_sources::cache::activer(&paths.cache_http());
    println!("Installation : {}", installation.display());
    println!(
        "Mode         : {}",
        if corriger {
            "ÉCRITURE"
        } else {
            "analyse seule — rien ne sera modifié"
        }
    );
    let replis = ygo_app::replis::connus(&paths);
    let noms = ygo_app::replis::noms_a_verifier(&replis);
    println!();
    println!("Replis connus : {}", replis.len());
    for r in replis.iter().take(10) {
        println!("  {}", r.destination);
    }
    if replis.len() > 10 {
        println!("  … et {} autre(s)", replis.len() - 10);
    }
    if noms.is_empty() {
        return Ok(true);
    }
    // Une requête pour 50 fichiers, sous quota et une à la fois.
    let client = ygo_sources::ClientHttp::new()?;
    let presents = ygo_sources::yugipedia::fichiers_existants(&client, &noms).await?;
    println!(
        "Yugipedia     : {} fichier(s) sur {} existent désormais ({} requête(s))",
        presents.len(),
        noms.len(),
        noms.len()
            .div_ceil(ygo_sources::yugipedia::TITRES_PAR_REQUETE)
    );
    for n in &presents {
        println!("  + {n}");
    }
    if corriger {
        let mut bilan = ygo_app::replis::Reprise::default();
        ygo_app::replis::rouvrir(&paths, &replis, &presents, &mut bilan)?;
        println!();
        println!(
            "{} rouvert(s), {} fichier(s) effacé(s) ; {} restent en repli jusqu'à la prochaine mise à jour.",
            bilan.presents, bilan.effaces, bilan.absents
        );
        if bilan.presents > 0 {
            println!("Les vraies images arriveront à la prochaine ouverture de leur classeur.");
        }
    } else if !presents.is_empty() {
        println!();
        println!("Relancer avec --corriger pour rouvrir ceux-là.");
    }
    Ok(true)
}

/// `etats-defaut` — pose l'état par défaut (Mint) et l'édition connue sur les
/// cartes **déjà possédées** qui n'en ont pas.
///
/// Les cartes qui entrent dans la collection depuis le 2026-10-01 les
/// reçoivent d'elles-mêmes ; celles d'avant, par cette commande. Rien de ce
/// qui est renseigné n'est touché, et une édition n'est posée que si la base
/// de cartes n'en connaît qu'une pour ce tirage. Sans `--corriger`, elle
/// **n'écrit rien**.
fn cmd_etats_defaut(installation: &Path, args: &[String]) -> anyhow::Result<bool> {
    let corriger = args.iter().any(|a| a == "--corriger");
    let seul = args
        .iter()
        .position(|a| a == "--classeur")
        .and_then(|i| args.get(i + 1))
        .map(|c| c.to_uppercase());

    let paths = Paths::depuis_racine(installation);
    println!("Installation : {}", installation.display());
    println!(
        "Mode         : {}",
        if corriger {
            "ÉCRITURE"
        } else {
            "analyse seule — rien ne sera modifié"
        }
    );
    println!(
        "Défaut       : état « {} », édition quand la base n'en connaît qu'une",
        ygo_app::exemplaires::QUALITE_PAR_DEFAUT
    );
    println!();
    println!(
        "{:10} {:>9} {:>8} {:>8} {:>10}",
        "classeur", "possédées", "état", "édition", "éd. ?"
    );

    let mut total = ygo_app::exemplaires::Completion::default();
    for classeur in accueil::lister(&paths, (3, 3)) {
        if seul.as_deref().is_some_and(|c| c != classeur.code) {
            continue;
        }
        let db = paths.classeur_db(&classeur.code);
        if !db.is_file() {
            continue;
        }
        let mut conn = rusqlite::Connection::open(&db)?;
        let defauts = ygo_app::exemplaires::Defauts::charger(&paths, &conn);
        let c = ygo_app::exemplaires::completer_classeur(&mut conn, &defauts, corriger)?;
        if c.possedees == 0 {
            continue;
        }
        println!(
            "{:10} {:>9} {:>8} {:>8} {:>10}",
            classeur.code, c.possedees, c.qualites, c.editions, c.editions_inconnues
        );
        total.possedees += c.possedees;
        total.qualites += c.qualites;
        total.editions += c.editions;
        total.editions_inconnues += c.editions_inconnues;
    }
    println!();
    println!(
        "{} possédée(s) : {} reçoivent l'état « {} », {} leur édition ; \
         {} restent sans édition (la base en connaît plusieurs, ou aucune)",
        total.possedees,
        total.qualites,
        ygo_app::exemplaires::QUALITE_PAR_DEFAUT,
        total.editions,
        total.editions_inconnues
    );
    if corriger {
        println!("Écrit.");
    } else if total.qualites + total.editions > 0 {
        println!("Relancer avec --corriger pour écrire.");
    }
    Ok(true)
}

/// `images-tirage` — pose l'image Yugipedia de chaque tirage dans les classeurs
/// déjà créés.
///
/// Les classeurs créés depuis le 2026-09-30 la reçoivent à la création ; ceux
/// d'avant, par cette commande. Sans `--corriger`, elle **n'écrit rien**. Seule
/// `card_image_url` change — ni possession, ni quantité, ni état, ni édition.
/// Les images elles-mêmes arrivent à la prochaine ouverture du classeur.
fn cmd_images_tirage(installation: &Path, args: &[String]) -> anyhow::Result<bool> {
    let corriger = args.iter().any(|a| a == "--corriger");
    let seul = args
        .iter()
        .position(|a| a == "--classeur")
        .and_then(|i| args.get(i + 1))
        .map(|c| c.to_uppercase());

    let paths = Paths::depuis_racine(installation);
    let reference = Priorites::charger(paths.rarity_config());
    let cardinfo = ygo_db::connexion::ouvrir_lecture_seule(paths.cardinfo_db())
        .context("cardinfo.db illisible")?;
    println!("Installation : {}", installation.display());
    println!(
        "Mode         : {}",
        if corriger {
            "ÉCRITURE"
        } else {
            "analyse seule — rien ne sera modifié"
        }
    );
    println!();
    println!(
        "{:10} {:>6} {:>7} {:>6} {:>8} {:>8} {:>8} {:>7} {:>7}",
        "classeur",
        "lignes",
        "à poser",
        "déjà",
        "artworks",
        "sans tir",
        "sans img",
        "ambigu",
        "rendues"
    );

    let mut total = images_tirage::Rapport::default();
    for classeur in accueil::lister(&paths, (3, 3)) {
        if seul.as_deref().is_some_and(|c| c != classeur.code) {
            continue;
        }
        let db = paths.classeur_db(&classeur.code);
        if !db.is_file() {
            continue;
        }
        let mut conn = rusqlite::Connection::open(&db)?;
        let r = images_tirage::analyser(&conn, &cardinfo, &reference)?;
        println!(
            "{:10} {:>6} {:>7} {:>6} {:>8} {:>8} {:>8} {:>7} {:>7}",
            classeur.code,
            r.lues,
            r.a_poser.len() - r.rendues,
            r.deja,
            r.servies_artworks,
            r.sans_tirage,
            r.sans_image,
            r.ambigues,
            r.rendues
        );
        if corriger && !r.a_poser.is_empty() {
            images_tirage::appliquer(&mut conn, &r)?;
        }
        total.lues += r.lues;
        total.a_poser.extend(r.a_poser);
        total.deja += r.deja;
        total.servies_artworks += r.servies_artworks;
        total.sans_tirage += r.sans_tirage;
        total.sans_image += r.sans_image;
        total.ambigues += r.ambigues;
        total.rendues += r.rendues;
    }
    println!();
    println!(
        "{} ligne(s) : {} avec l'image de leur tirage, {} déjà servies par la passe artworks, \
         {} gardent leur image actuelle",
        total.lues,
        total.couvertes(),
        total.servies_artworks,
        total.lues - total.couvertes() - total.servies_artworks
    );
    if corriger {
        println!("Écrit. Les images arriveront à la prochaine ouverture de chaque classeur.");
    } else if !total.a_poser.is_empty() {
        println!("Relancer avec --corriger pour écrire.");
    }
    Ok(true)
}

/// `images` — télécharge les images manquantes.
///
/// Sans `--telecharger`, la commande **ne va pas sur le réseau** : elle dit ce
/// qui manque. C'est la forme par défaut parce qu'un classeur neuf peut
/// demander plusieurs centaines de requêtes, et qu'il vaut mieux le savoir
/// avant.
async fn cmd_images(installation: &Path, args: &[String]) -> anyhow::Result<bool> {
    let telecharger = args.iter().any(|a| a == "--telecharger");
    let seul = args
        .iter()
        .position(|a| a == "--classeur")
        .and_then(|i| args.get(i + 1))
        .map(|c| c.to_uppercase());

    let paths = Paths::depuis_racine(installation);
    let config = Config::charger(paths.app_config());
    let source = config.source_image();
    let client = ygo_sources::ClientHttp::new()?;
    let telechargeur = ygo_images::Telechargeur::nouveau(&client, &paths.image_par_defaut());

    println!("Installation : {}", installation.display());
    println!("Source       : {source:?}");
    println!(
        "Substitut    : {}",
        if telechargeur.signature().is_some() {
            "img/notfound.jpg présent — les substituts posés seront retentés"
        } else {
            "img/notfound.jpg ABSENT — un échec laissera le fichier absent"
        }
    );
    println!(
        "Mode         : {}",
        if telecharger {
            "TÉLÉCHARGEMENT"
        } else {
            "analyse seule — aucune requête réseau"
        }
    );
    println!();

    let classeurs = accueil::lister(&paths, (3, 3));
    let mut total_manquantes = 0_usize;
    let mut bilan_total = ygo_images::Bilan::default();

    for classeur in &classeurs {
        if seul.as_deref().is_some_and(|c| c != classeur.code) {
            continue;
        }
        if !paths.classeur_db(&classeur.code).is_file() {
            continue;
        }
        let (cibles, deja, lignes) =
            images::a_faire(&paths, &classeur.code, source, &telechargeur)?;
        if cibles.is_empty() {
            continue;
        }
        total_manquantes += cibles.len();
        println!(
            "{} — {lignes} ligne(s), {deja} image(s) présente(s), {} manquante(s)",
            classeur.code,
            cibles.len()
        );
        for cible in cibles.iter().take(5) {
            println!("     {}", cible.url_primaire);
        }
        if cibles.len() > 5 {
            println!("     … et {} autre(s)", cibles.len() - 5);
        }

        if telecharger {
            let issue = images::passe(&paths, &client, &classeur.code, source, |faites, total| {
                if faites % 10 == 0 || faites == total {
                    println!("     {faites}/{total}");
                }
            })
            .await?;
            println!(
                "  {} téléchargée(s), {} indisponible(s)",
                issue.bilan.reussies, issue.bilan.echecs
            );
            bilan_total.reussies += issue.bilan.reussies;
            bilan_total.echecs += issue.bilan.echecs;
        }
        println!();
    }

    if total_manquantes == 0 {
        println!("Toutes les images sont présentes.");
    } else if telecharger {
        println!(
            "{} téléchargée(s), {} indisponible(s)",
            bilan_total.reussies, bilan_total.echecs
        );
    } else {
        println!("{total_manquantes} image(s) manquante(s).");
        println!("Relancer avec --telecharger pour les récupérer.");
    }
    Ok(true)
}

/// `doublons` — retire les lignes que la passe artworks a insérées à tort.
///
/// Sans `--corriger`, la commande **ne supprime rien**. Vu ce qu'elle retire
/// — 751 lignes sur la V1.0.4 — c'est la forme par défaut qui s'impose.
/// `anomalies` — les artworks manquants d'un classeur.
///
/// Trois gestes, dans l'ordre où ils se pensent : **scanner** relit
/// `cardinfo.db` et remplit la table ; sans `--corriger`, la commande ne fait
/// que dire ce qu'elle ferait ; avec, elle écrit dans les classeurs.
fn cmd_anomalies(installation: &Path, args: &[String]) -> anyhow::Result<bool> {
    let scanner = args.iter().any(|a| a == "--scanner");
    let corriger = args.iter().any(|a| a == "--corriger");
    let detail = args.iter().any(|a| a == "--detail");
    let seul = args
        .iter()
        .position(|a| a == "--classeur")
        .and_then(|i| args.get(i + 1))
        .map(|c| c.to_uppercase());

    let paths = Paths::depuis_racine(installation);
    println!("Installation : {}", installation.display());
    println!(
        "Mode         : {}",
        if corriger {
            "ÉCRITURE dans les classeurs"
        } else {
            "lecture seule — rien ne sera modifié"
        }
    );
    println!();

    if scanner {
        let bilan = anomalies::scanner(&paths)?;
        println!(
            "Scan : {} détectée(s), {} nouvelle(s), {} retirée(s) — {} en base",
            bilan.detectees, bilan.ajoutees, bilan.retirees, bilan.total
        );
        println!();
    }

    let liste = anomalies::lire(&paths, seul.as_deref())?;
    if liste.is_empty() {
        println!("Aucune anomalie connue. Lancez avec --scanner pour en chercher.");
        return Ok(true);
    }

    let mut par_classeur: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for a in &liste {
        let e = par_classeur.entry(a.set_code_prefix.as_str()).or_default();
        e.0 += 1;
        if a.corrige {
            e.1 += 1;
        }
    }
    for (classeur, (total, corrigees)) in &par_classeur {
        println!("{classeur:10} {total:4} anomalie(s), {corrigees} déjà corrigée(s)");
        if detail {
            for a in liste.iter().filter(|a| a.set_code_prefix == *classeur) {
                println!(
                    "  {}{} {} · art {} · {}",
                    if a.corrige { "✓ " } else { "  " },
                    a.missing_set_code,
                    a.missing_set_rarity,
                    a.art_index,
                    a.nom
                );
            }
        }
    }
    println!();

    if !corriger {
        let restantes = liste.iter().filter(|a| !a.corrige).count();
        println!(
            "{} anomalie(s) à corriger. Relancez avec --corriger.",
            restantes
        );
        return Ok(true);
    }

    let mut ajoutees = 0_usize;
    let mut deja = 0_usize;
    let mut sans_temoin = 0_usize;
    let mut touches: BTreeSet<String> = BTreeSet::new();
    for a in &liste {
        match anomalies::corriger(&paths, a)? {
            anomalies::Correction::Ajoutee { classeur, .. } => {
                ajoutees += 1;
                touches.insert(classeur);
            }
            anomalies::Correction::DejaPresente => deja += 1,
            anomalies::Correction::SansTemoin | anomalies::Correction::ClasseurAbsent => {
                sans_temoin += 1;
            }
        }
    }
    println!(
        "{ajoutees} ligne(s) ajoutée(s) dans {} classeur(s) : {}",
        touches.len(),
        touches.into_iter().collect::<Vec<_>>().join(", ")
    );
    if deja > 0 {
        println!("{deja} déjà présente(s) — rien à faire.");
    }
    if sans_temoin > 0 {
        println!("{sans_temoin} sans ligne d'où hériter — non corrigée(s).");
    }
    Ok(true)
}

fn cmd_doublons(installation: &Path, args: &[String]) -> anyhow::Result<bool> {
    let corriger = args.iter().any(|a| a == "--corriger");
    let seul = args
        .iter()
        .position(|a| a == "--classeur")
        .and_then(|i| args.get(i + 1))
        .map(|c| c.to_uppercase());

    let paths = Paths::depuis_racine(installation);
    let reference = Priorites::charger(paths.rarity_config());
    println!("Installation : {}", installation.display());
    println!(
        "Référence    : {} raretés dans rarity_config.json",
        reference.len()
    );
    if reference.is_empty() {
        anyhow::bail!(
            "rarity_config.json vide ou illisible — sans la liste des Options, \
             aucune ligne ne peut être déclarée canonique, et supprimer à l'aveugle \
             est hors de question"
        );
    }
    println!(
        "Mode         : {}",
        if corriger {
            "SUPPRESSION"
        } else {
            "analyse seule — rien ne sera supprimé"
        }
    );
    println!();

    let classeurs = accueil::lister(&paths, (3, 3));
    let mut total_suppressions = 0_usize;
    let mut total_bloquees = 0_usize;

    for classeur in &classeurs {
        if seul.as_deref().is_some_and(|c| c != classeur.code) {
            continue;
        }
        let db = paths.classeur_db(&classeur.code);
        if !db.is_file() {
            continue;
        }
        let mut conn = rusqlite::Connection::open(&db)?;
        let rapport = doublons::analyser(&conn, &reference)?;
        if rapport.vide() && rapport.bloquees.is_empty() && rapport.sans_temoin.is_empty() {
            continue;
        }

        println!(
            "{} — {} ligne(s) → {} après",
            classeur.code,
            rapport.lues,
            rapport.lues - rapport.suppressions.len()
        );
        for (libelle, n) in rapport.par_libelle() {
            println!("  {n:5}  {libelle}  →  supprimée, doublée par une ligne canonique");
        }
        for (libelle, n) in &rapport.sans_temoin {
            println!("  {n:5}  {libelle}  →  GARDÉE, elle ne double personne");
        }
        for b in &rapport.bloquees {
            println!(
                "  ATTENTION  {} « {} » rowid {} porte une quantité de {} — laissée en place",
                b.set_code, b.rarity, b.rowid, b.quantite
            );
        }
        total_suppressions += rapport.suppressions.len();
        total_bloquees += rapport.bloquees.len();

        if corriger && !rapport.vide() {
            let supprimees = doublons::appliquer(&mut conn, &rapport)?;
            println!("  {supprimees} ligne(s) supprimée(s)");
        }
        println!();
    }

    println!(
        "{} classeur(s), {total_suppressions} ligne(s) en trop, {total_bloquees} bloquée(s) par une quantité",
        classeurs.len()
    );
    if !corriger && total_suppressions > 0 {
        println!();
        println!("Relancer avec --corriger pour supprimer.");
        println!("À faire ensuite, dans cet ordre : `raretes --corriger`, puis `artworks`.");
    }
    Ok(true)
}

/// Une ligne de classeur, réduite à ce que la passe artworks peut changer.
type EtatLigne = (i64, String, String, i64, i64, String);

fn etat_lignes(conn: &rusqlite::Connection) -> anyhow::Result<Vec<EtatLigne>> {
    let mut requete = conn.prepare(
        "SELECT rowid, COALESCE(set_code, ''), COALESCE(rarity, ''), \
                COALESCE(extended_art, 0), COALESCE(card_image_id, 0), \
                COALESCE(card_image_uuid, '') \
         FROM cards ORDER BY rowid",
    )?;
    let lignes = requete
        .query_map([], |l| {
            Ok((
                l.get(0)?,
                l.get(1)?,
                l.get(2)?,
                l.get(3)?,
                l.get(4)?,
                l.get(5)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(lignes)
}

/// Dit, ligne par ligne, ce que la passe a changé.
///
/// Trois catégories, et c'est la troisième qui compte :
/// - **créée** : une ligne de variante que le classeur n'avait pas ;
/// - **illustration posée** : une ligne qui n'en avait pas de Yugipedia ;
/// - **illustration REMPLACÉE** : une ligne qui en avait déjà une autre. Sur un
///   classeur fraîchement dédoublonné, cette catégorie doit rester vide.
fn detailler(conn: &rusqlite::Connection, avant: &[EtatLigne]) -> anyhow::Result<()> {
    use std::collections::HashMap;

    let apres = etat_lignes(conn)?;
    let index: HashMap<i64, &EtatLigne> = avant.iter().map(|l| (l.0, l)).collect();

    let mut creees = Vec::new();
    let mut posees = Vec::new();
    let mut remplacees = Vec::new();

    for ligne in &apres {
        let (rowid, set_code, rarity, ext, image, _uuid) = ligne;
        match index.get(rowid) {
            None => creees.push(ligne),
            Some(vieille) if vieille.4 != *image => {
                // Un identifiant négatif désigne un artwork Yugipedia.
                if vieille.4 < 0 {
                    remplacees.push((ligne, vieille.4));
                } else {
                    posees.push(ligne);
                }
            }
            Some(vieille) if vieille.3 != *ext => posees.push(ligne),
            Some(_) => {}
        }
        let _ = (set_code, rarity);
    }

    println!();
    println!("  ── détail ──");
    println!("  lignes créées                {}", creees.len());
    for l in creees.iter().take(20) {
        println!(
            "     {} {} {}  image {}",
            l.1,
            if l.3 == 1 {
                "[Overframe]"
            } else {
                "[normal]  "
            },
            l.2,
            l.4
        );
    }
    if creees.len() > 20 {
        println!("     … et {} autre(s)", creees.len() - 20);
    }

    println!("  illustrations posées         {}", posees.len());
    let mut par_numero: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for l in &posees {
        *par_numero.entry(l.1.as_str()).or_insert(0) += 1;
    }
    for (numero, n) in &par_numero {
        println!("     {numero}  {n} ligne(s)");
    }

    println!("  illustrations REMPLACÉES     {}", remplacees.len());
    for (l, ancienne) in remplacees.iter().take(20) {
        println!("     {} {}  {} → {}", l.1, l.2, ancienne, l.4);
    }
    if remplacees.is_empty() {
        println!("     (aucune : rien d'existant n'a été écrasé)");
    }
    Ok(())
}

/// Retrouve la liste des Options depuis le chemin d'un classeur.
///
/// Un classeur vit en `<installation>/bdd/classeur_creer/<CODE>/<CODE>.db` ;
/// on remonte donc jusqu'à trouver un `bdd/rarity_config.json`. À défaut, une
/// table vide : la canonisation continue alors de fonctionner par la table
/// d'abréviations et le référentiel, elle perd seulement l'arbitrage des
/// orthographes.
fn raretes_pour_classeur(classeur: &Path) -> Priorites {
    let mut dossier = classeur.parent();
    while let Some(d) = dossier {
        let candidat = d.join("bdd").join("rarity_config.json");
        if candidat.is_file() {
            return Priorites::charger(candidat);
        }
        dossier = d.parent();
    }
    Priorites::default()
}

fn cmd_comparer(reference: &Path, candidate: &Path) -> anyhow::Result<bool> {
    use ygo_db::init::TABLES_HORS_INIT;

    let a = connexion::ouvrir_lecture_seule(reference)
        .with_context(|| format!("ouverture de {}", reference.display()))?;
    let b = connexion::ouvrir_lecture_seule(candidate)
        .with_context(|| format!("ouverture de {}", candidate.display()))?;

    println!("Référence : {}", reference.display());
    println!("Candidate : {}", candidate.display());
    println!();

    let mut toutes: Vec<String> = tables_de(&a)?.into_iter().chain(tables_de(&b)?).collect();
    toutes.sort();
    toutes.dedup();

    println!(
        "{:<24} {:>10} {:>10}   écart",
        "table", "référence", "candidate"
    );
    println!("{}", "─".repeat(60));

    let mut ecarts_init: Vec<&str> = Vec::new();
    let mut hors_init_absentes: Vec<&str> = Vec::new();

    for table in &toutes {
        let nom = table.as_str();
        let hors_init = TABLES_HORS_INIT.contains(&nom);
        let na = compter(&a, nom);
        let nb = compter(&b, nom);

        let ecart = match (na, nb) {
            (Some(x), Some(y)) if x == y => "=".to_owned(),
            (Some(x), Some(y)) => {
                if !hors_init {
                    ecarts_init.push(nom);
                }
                format!("{:+}", y - x)
            }
            (Some(_), None) if hors_init => {
                // Une base fraîchement initialisée n'a pas ces tables : elles
                // sont créées par les fonctionnalités qui les possèdent.
                hors_init_absentes.push(nom);
                "hors init".to_owned()
            }
            (Some(_), None) => {
                ecarts_init.push(nom);
                "absente cand.".to_owned()
            }
            (None, Some(_)) => {
                if !hors_init {
                    ecarts_init.push(nom);
                }
                "absente réf.".to_owned()
            }
            (None, None) => "—".to_owned(),
        };

        let ta = na.map_or_else(|| "—".to_owned(), |n| n.to_string());
        let tb = nb.map_or_else(|| "—".to_owned(), |n| n.to_string());
        println!("{nom:<24} {ta:>10} {tb:>10}   {ecart}");
    }

    println!();
    if !hors_init_absentes.is_empty() {
        println!(
            "Hors périmètre : {} — créée(s) après l'initialisation, par la",
            hors_init_absentes.join(", ")
        );
        println!("détection d'anomalies, la passe Overframe et les artworks Yugipedia.");
    }

    // Un écart sur `set_prints` alors que la passe Overframe n'a pas tourné côté
    // candidate s'explique de lui-même : c'est elle qui ajoute ces lignes après
    // l'initialisation. On le signale, on ne le compte pas comme une divergence
    // — mais uniquement si c'est le SEUL écart, sans quoi on masquerait un vrai
    // problème derrière une explication commode.
    let overframe_manquant = hors_init_absentes.contains(&"overframe_sync");
    let ecart_prints_seul = ecarts_init == ["set_prints"] && overframe_manquant;

    if ecart_prints_seul {
        let delta = compter(&b, "set_prints").unwrap_or(0) - compter(&a, "set_prints").unwrap_or(0);
        println!();
        println!("Note : l'écart de {delta:+} sur `set_prints` correspond aux tirages ajoutés par");
        println!("la passe Overframe, qui n'a pas tourné sur la candidate — sa table de suivi");
        println!("`overframe_sync` y est absente. Ce n'est donc pas une divergence de");
        println!("l'initialisation, mais une fonctionnalité qui reste à porter.");
    }

    println!();
    let conclusion = if ecarts_init.is_empty() {
        "Conclusion : le schéma d'initialisation concorde intégralement."
    } else if ecart_prints_seul {
        "Conclusion : l'initialisation concorde ; seul subsiste l'apport de la passe Overframe."
    } else {
        "Conclusion : au moins une table du schéma d'initialisation diverge."
    };
    println!("{conclusion}");
    Ok(ecarts_init.is_empty() || ecart_prints_seul)
}

fn tables_de(conn: &rusqlite::Connection) -> anyhow::Result<Vec<String>> {
    let mut requete = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )?;
    let noms = requete
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(noms)
}

fn compter(conn: &rusqlite::Connection, table: &str) -> Option<i64> {
    conn.query_row(&format!("SELECT count(*) FROM \"{table}\""), [], |r| {
        r.get(0)
    })
    .ok()
}

// ─────────────────────────────────────────────────────────────────────────────
// overframe — complétion des tirages Overframe
// ─────────────────────────────────────────────────────────────────────────────

/// Applique la passe Overframe à une `cardinfo.db`.
///
/// **Écrit dans la base indiquée** — sauf `--simuler`, qui déroule tout puis
/// annule la transaction. C'est la commande qui permet de vérifier, sur les
/// données réelles, que le portage produit les 36 tirages qui manquaient à la
/// base construite par le Rust.
async fn cmd_overframe(base: &Path, args: &[String]) -> anyhow::Result<bool> {
    let force = args.iter().any(|a| a == "--force");
    let simuler = args.iter().any(|a| a == "--simuler");

    if !base.is_file() {
        bail!("base introuvable : {}", base.display());
    }
    println!("Base      : {}", base.display());
    println!(
        "Mode      : {}{}",
        if force {
            "forcé (révision ignorée)"
        } else {
            "idempotent (révision respectée)"
        },
        if simuler { ", SIMULATION" } else { "" }
    );

    let client = ygo_sources::ClientHttp::new()?;
    let mut conn = rusqlite::Connection::open(base)?;
    let horodatage = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let tx = conn.transaction()?;

    let rapports = overframe::enrichir_whitelist(&tx, &client, force, &horodatage).await;

    let mut total_ajoutes = 0usize;
    let mut total_corriges = 0usize;
    let mut echec = false;
    println!();
    for rapport in &rapports {
        match &rapport.issue {
            Ok(overframe::Issue::Fait { bilan, revid }) => {
                total_ajoutes += bilan.ajoutes;
                total_corriges += bilan.corriges;
                println!(
                    "  {:9} +{} tirage(s), {} drapeau(x) corrigé(s)  (révision {revid})",
                    rapport.prefixe, bilan.ajoutes, bilan.corriges
                );
            }
            Ok(overframe::Issue::DejaAJour { revid }) => {
                println!(
                    "  {:9} déjà à jour (révision {revid}) — relancer avec --force pour réappliquer",
                    rapport.prefixe
                );
            }
            Ok(overframe::Issue::AucunOverframe { revid }) => {
                println!(
                    "  {:9} aucun tirage Overframe déclaré (révision {revid})",
                    rapport.prefixe
                );
            }
            Ok(overframe::Issue::PageIntrouvable) => {
                println!("  {:9} aucune page « Set Card Lists »", rapport.prefixe);
            }
            Ok(overframe::Issue::AbsentCardinfo) => {
                println!("  {:9} absent de cette base", rapport.prefixe);
            }
            Err(e) => {
                echec = true;
                println!("  {:9} ÉCHEC — {e}", rapport.prefixe);
            }
        }
    }

    if simuler {
        tx.rollback()?;
        println!("\nSimulation : rien n'a été écrit.");
    } else {
        tx.commit()?;
    }
    println!("\nTotal : +{total_ajoutes} tirage(s), {total_corriges} drapeau(x) corrigé(s).");
    Ok(!echec)
}

// ─────────────────────────────────────────────────────────────────────────────
// creer — la création complète d'un classeur
// ─────────────────────────────────────────────────────────────────────────────

/// Crée le classeur d'un set dans l'installation indiquée.
///
/// C'est le `create_classeur` du Python : cascade local/API, écriture, puis
/// passe artworks. Un classeur déjà peuplé n'est jamais retouché.
async fn cmd_creer(installation: &Path, code: &str, args: &[String]) -> anyhow::Result<bool> {
    let sans_artworks = args.iter().any(|a| a == "--sans-artworks");
    let paths = Paths::depuis_racine(installation);
    let base = paths.cardinfo_db();
    if !base.is_file() {
        bail!("cardinfo.db introuvable : {}", base.display());
    }
    let code = code.trim().to_uppercase();
    println!("Installation : {}", installation.display());
    println!("Classeur     : {code}");
    if sans_artworks {
        println!("Artworks     : passe désactivée");
    }

    let client = ygo_sources::ClientHttp::new()?;
    let issue = creation::creer(
        &paths,
        &client,
        &code,
        !sans_artworks,
        &creation::Greffons::default(),
    )
    .await?;

    println!();
    match issue {
        creation::Issue::DejaExistant => {
            println!("  Le classeur existe déjà et contient des cartes — rien n'a été touché.");
        }
        creation::Issue::Cree {
            source,
            lignes,
            artworks,
            raretes: bilan_raretes,
        } => {
            println!(
                "  {lignes} ligne(s) écrite(s) dans {}",
                paths.classeur_db(&code).display()
            );
            println!(
                "  source           {}",
                match source {
                    creation::Source::Local => "cardinfo.db",
                    creation::Source::Api => "YGOPRODeck",
                    creation::Source::LocalApresEchecApi =>
                        "cardinfo.db — APRÈS ÉCHEC de YGOPRODeck, données possiblement partielles",
                }
            );
            match artworks {
                creation::Artworks::Faite(bilan) => {
                    println!(
                        "  artworks         {} illustration(s), {} tirage(s) ajouté(s), \
                         {} drapeau(x), {} numéro(s) absent(s)",
                        bilan.illustrations, bilan.ajoutes, bilan.flags, bilan.absents
                    );
                }
                creation::Artworks::Ignoree(raison) => {
                    println!("  artworks         IGNORÉE — {raison}");
                    println!(
                        "                   (le classeur est créé ; relancez `artworks` plus tard)"
                    );
                }
                creation::Artworks::NonDemandee => {}
            }
            if bilan_raretes.corrigees > 0 {
                println!(
                    "  raretés          {} libellé(s) ramené(s) à la forme des Options",
                    bilan_raretes.corrigees
                );
            }
            for (libelle, n) in &bilan_raretes.inconnues {
                println!(
                    "  raretés          {n} ligne(s) « {libelle} » — non reconnu, laissé tel quel"
                );
            }
        }
    }
    Ok(true)
}

// ─────────────────────────────────────────────────────────────────────────────
// artworks — alignement d'un classeur sur la Set list
// ─────────────────────────────────────────────────────────────────────────────

/// Exécute la passe artworks sur le classeur indiqué.
///
/// Cette commande ÉCRIT dans le classeur. `--simuler` annule la transaction à
/// la fin : le compte rendu est le vrai, la base est intacte.
async fn cmd_artworks(classeur: &Path, args: &[String]) -> anyhow::Result<bool> {
    let simuler = args.iter().any(|a| a == "--simuler");
    let langue = args
        .iter()
        .position(|a| a == "--langue")
        .and_then(|i| args.get(i + 1))
        .map(|l| l.to_uppercase())
        .unwrap_or_default();

    if !classeur.is_file() {
        bail!("classeur introuvable : {}", classeur.display());
    }
    println!("Classeur : {}", classeur.display());
    if !langue.is_empty() {
        println!("Langue   : {langue} (forcée)");
    }
    if simuler {
        println!("Mode     : SIMULATION — rien ne sera écrit");
    }

    let detail = args.iter().any(|a| a == "--detail");

    let client = ygo_sources::ClientHttp::new()?;
    let mut conn = rusqlite::Connection::open(classeur)?;
    let tx = conn.transaction()?;

    // L'état d'avant, pour dire ligne par ligne ce que la passe a changé.
    // Sans ça, un bilan chiffré laisse la question ouverte : « 49
    // illustrations pour 14 lignes créées » ne dit pas sur QUOI les 35 autres
    // se posent, et une illustration de variante posée sur la ligne du tirage
    // normal serait une régression invisible au décompte.
    let avant = if detail {
        etat_lignes(&tx)?
    } else {
        Vec::new()
    };

    let raretes = raretes_pour_classeur(classeur);
    println!(
        "Raretés  : {} entrée(s) dans rarity_config.json",
        raretes.len()
    );
    let issue = artworks::passe(&tx, &client, &langue, &raretes).await?;
    println!();
    let succes = match &issue {
        artworks::Issue::Fait {
            bilan,
            revid,
            titre,
        } => {
            println!("  page             {titre}");
            match revid {
                Some(r) => println!("  révision         {r}"),
                None => println!("  révision         inconnue"),
            }
            println!("  tirages ajoutés  {}", bilan.ajoutes);
            println!("  illustrations    {}", bilan.illustrations);
            println!("  drapeaux montés  {}", bilan.flags);
            println!(
                "  images réutilisées d'une autre rareté  {}",
                bilan.reutilisees
            );
            println!("  numéros absents du classeur            {}", bilan.absents);
            if detail {
                detailler(&tx, &avant)?;
            }
            if bilan.illustrations == 0 && bilan.ajoutes == 0 && bilan.flags == 0 {
                println!(
                    "\n  Rien à faire — soit la passe a déjà tourné (les lignes servies\n  \
                     sont protégées), soit ce set n'a aucune variante d'illustration."
                );
            }
            true
        }
        artworks::Issue::ClasseurVide => {
            println!("  Classeur vide ou sans nom de set — rien à aligner.");
            true
        }
        artworks::Issue::PageIntrouvable => {
            println!("  Aucune page « Set Card Lists » pour ce set.");
            false
        }
        artworks::Issue::StructureSterile { titre } => {
            println!("  {titre} — aucun tirage exploitable dans la page.");
            false
        }
    };

    if simuler {
        tx.rollback()?;
        println!("\nSimulation : rien n'a été écrit.");
    } else {
        tx.commit()?;
    }
    Ok(succes)
}

// ─────────────────────────────────────────────────────────────────────────────
// classeur — construction des lignes depuis cardinfo.db
// ─────────────────────────────────────────────────────────────────────────────

/// Construit les lignes d'un classeur et rend compte de la décision.
///
/// Aucune requête réseau : c'est le chemin local, celui que l'application tente
/// toujours en premier. Sert à comparer, sur les données réelles, ce que le
/// portage produit à ce que produit la V1.0.4.
fn cmd_classeur(installation: &Path, code: &str, args: &[String]) -> anyhow::Result<bool> {
    let code = code.trim().to_uppercase();
    let paths = Paths::depuis_racine(installation);
    let base = paths.cardinfo_db();
    if !base.is_file() {
        bail!("cardinfo.db introuvable : {}", base.display());
    }
    println!("Base     : {}", base.display());
    println!("Classeur : {code}");

    let conn = connexion::ouvrir_lecture_seule(&base)?;
    let lignes = match creation::construire_lignes_locales(&conn, &code) {
        Ok(lignes) => lignes,
        Err(e) => {
            println!("\nConstruction locale impossible : {e}");
            println!(
                "Décision : {:?}",
                creation::decider(false, &code, Err(e.to_string()))
            );
            return Ok(false);
        }
    };

    let verdict = creation::locales_semblent_incompletes(&lignes);
    let overframe = lignes.iter().filter(|l| l.extended_art != 0).count();
    println!("\n{} ligne(s)", lignes.len());
    println!(
        "  {} carte(s) distincte(s), moyenne {:.2} rareté(s)/carte{}",
        verdict.cartes_uniques,
        verdict.moyenne,
        if verdict.suspect {
            "  — SOUS LE SEUIL, l'application basculerait sur YGOPRODeck"
        } else {
            ""
        }
    );
    if overframe > 0 {
        println!("  {overframe} tirage(s) Overframe");
    }
    println!(
        "  décision : {:?}",
        creation::decider(false, &code, Ok(&lignes))
    );

    println!("\nPremières lignes :");
    for l in lignes.iter().take(5) {
        let nom = if l.name.is_empty() {
            &l.name_fr
        } else {
            &l.name
        };
        println!(
            "  {:>4}  {:14} {:24} {nom}",
            l.sort_order, l.set_code, l.rarity
        );
    }

    if let Some(i) = args.iter().position(|a| a == "--ecrire") {
        let dest = PathBuf::from(args.get(i + 1).context("chemin de destination attendu")?);
        let n = creation::ecrire_classeur(&dest, &lignes, &ygo_app::classeur::Ecarts::default())?;
        println!("\n{n} ligne(s) écrite(s) dans {}", dest.display());
    }
    Ok(true)
}

// ─────────────────────────────────────────────────────────────────────────────
// importer / exporter / inventaire
// ─────────────────────────────────────────────────────────────────────────────

/// L'argument qui suit `nom`, s'il existe.
fn valeur<'a>(args: &'a [String], nom: &str) -> Option<&'a String> {
    args.iter()
        .position(|a| a == nom)
        .and_then(|i| args.get(i + 1))
}

async fn cmd_importer(
    installation: &Path,
    fichier: &Path,
    args: &[String],
) -> anyhow::Result<bool> {
    let ecrire = args.iter().any(|a| a == "--ecrire");
    let creer_classeurs = args.iter().any(|a| a == "--creer-classeurs");
    let detail = args.iter().any(|a| a == "--detail");
    let seul = valeur(args, "--classeur").map(|c| c.to_uppercase());

    let paths = Paths::depuis_racine(installation);
    let lecture = ygo_app::scanflip::lire(fichier)?;
    println!("Installation : {}", installation.display());
    println!("Fichier      : {}", fichier.display());
    println!(
        "Mode         : {}",
        if ecrire {
            "ÉCRITURE"
        } else {
            "analyse seule — rien ne sera modifié"
        }
    );
    println!();

    let mut lignes = lecture.lignes;
    if let Some(code) = &seul {
        lignes.retain(|l| &l.extension == code);
    }
    println!("{} ligne(s) lue(s)", lignes.len());
    for i in &lecture.illisibles {
        println!("  ligne {} illisible : {}", i.numero, i.raison);
    }

    // Créer AVANT d'apparier : un classeur absent fait refuser toutes ses
    // lignes, et le créer ensuite obligerait à tout recommencer.
    let manquants = ygo_app::import::classeurs_manquants(&paths, &lignes);
    if !manquants.is_empty() {
        println!(
            "\n{} classeur(s) du CSV absent(s) de l'installation : {}",
            manquants.len(),
            manquants.join(", ")
        );
        if creer_classeurs {
            anyhow::ensure!(
                ecrire,
                "--creer-classeurs écrit dans l'installation : ajoutez --ecrire"
            );
            let client = ygo_sources::ClientHttp::new()?;
            for code in &manquants {
                print!("  {code} … ");
                use std::io::Write as _;
                std::io::stdout().flush().ok();
                match ygo_app::creation::creer(
                    &paths,
                    &client,
                    code,
                    true,
                    &ygo_app::creation::Greffons::default(),
                )
                .await
                {
                    Ok(ygo_app::creation::Issue::Cree { lignes, .. }) => {
                        println!("créé, {lignes} ligne(s)");
                    }
                    Ok(ygo_app::creation::Issue::DejaExistant) => println!("existait déjà"),
                    Err(e) => println!("ÉCHEC : {e}"),
                }
            }
        } else {
            println!("  (relancez avec --creer-classeurs --ecrire pour les créer)");
        }
        println!();
    }

    let bases = ygo_app::import::bases_pour(&paths, &lignes);
    let rapport = ygo_app::import::planifier(&lignes, &bases);

    println!(
        "{} appariée(s), {} refusée(s) — {} ligne(s) de classeur à écrire",
        rapport.appariees(),
        rapport.refusees.len(),
        rapport.ecritures.len()
    );

    if !rapport.refusees.is_empty() {
        println!("\nRefusées :");
        let mut par_cause: std::collections::BTreeMap<&str, Vec<&ygo_app::import::Refusee>> =
            std::collections::BTreeMap::new();
        for r in &rapport.refusees {
            par_cause.entry(r.refus.libelle()).or_default().push(r);
        }
        for (cause, lignes) in par_cause {
            println!("  {:5}  {cause}", lignes.len());
            if detail {
                for r in lignes {
                    println!(
                        "         ligne {:4}  {} {} artwork {} — {}",
                        r.numero, r.code, r.rarete, r.artwork, r.nom
                    );
                }
            }
        }
    }

    let avec_perte = rapport.fusions_avec_perte().count();
    if avec_perte > 0 {
        println!(
            "\n{avec_perte} fusion(s) effacent une information que la base ne sait pas garder :"
        );
        for f in rapport.fusions_avec_perte() {
            let mut quoi = Vec::new();
            if f.perte.langue {
                quoi.push("langue");
            }
            println!(
                "  {} {} — lignes {:?} sur une seule ligne de base ({})",
                f.classeur,
                f.code,
                f.numeros,
                quoi.join(", ")
            );
        }
    }

    if ecrire {
        let bilan = ygo_app::import::appliquer(&paths, &rapport)?;
        println!(
            "\n{} ligne(s) écrite(s) dans {} classeur(s) : {}",
            bilan.ecrites,
            bilan.classeurs.len(),
            bilan.classeurs.join(", ")
        );
    } else {
        println!("\nRien n'a été modifié. Relancez avec --ecrire pour appliquer.");
    }
    Ok(true)
}

fn cmd_exporter(installation: &Path, fichier: &Path, args: &[String]) -> anyhow::Result<bool> {
    let langue = if args.iter().any(|a| a == "--anglais") {
        ygo_app::export::Langue::Anglais
    } else {
        ygo_app::export::Langue::Francais
    };
    let seul = valeur(args, "--classeur").map(|c| c.to_uppercase());

    let paths = Paths::depuis_racine(installation);
    let mut classeurs = paths.classeurs_existants();
    if let Some(code) = &seul {
        classeurs.retain(|c| c == code);
        anyhow::ensure!(!classeurs.is_empty(), "classeur {code} introuvable");
    }
    let (classeurs, ecartes) = ygo_app::export::ecarter_ocg(&classeurs);

    let possedees = ygo_app::export::possedees_de(&paths, &classeurs)?;
    // Les rangs se lisent sur la base entière des classeurs, pas sur les
    // seules cartes possédées : c'est la même numérotation que l'import.
    let rangs = ygo_app::export::rangs(&ygo_app::export::bases_de(&paths, &classeurs)?);
    let lignes = ygo_app::export::lignes(&possedees, langue, &rangs);
    ygo_app::scanflip::ecrire(fichier, &lignes)?;
    let bilan = ygo_app::export::bilan(&lignes, ecartes);

    println!(
        "{} ligne(s), {} exemplaire(s), {} classeur(s) → {}",
        bilan.lignes,
        bilan.exemplaires,
        classeurs.len(),
        fichier.display()
    );
    if !bilan.ecartes.is_empty() {
        println!(
            "{} classeur(s) OCG écarté(s) : {} — leur code est natif, mais la \
             colonne Langue du format est unique pour tout le fichier",
            bilan.ecartes.len(),
            bilan.ecartes.join(", ")
        );
    }
    if !bilan.raretes_refusees.is_empty() {
        println!(
            "ATTENTION — {} rareté(s) hors des règles d'import de Scanflip : {}.\n\
             Ces lignes seront rejetées à l'arrivée.",
            bilan.raretes_refusees.len(),
            bilan.raretes_refusees.join(", ")
        );
    }
    if bilan.sans_etat > 0 {
        println!(
            "{} ligne(s) sortent sans état — aucune colonne d\'édition remplie.\n\
             Pour le renseigner en masse : ygo-cli etat <installation> NM --sans-etat --ecrire",
            bilan.sans_etat
        );
    }
    Ok(true)
}

fn cmd_inventaire(installation: &Path, args: &[String]) -> anyhow::Result<bool> {
    let paths = Paths::depuis_racine(installation);
    let filtre = ygo_app::inventaire::Filtre {
        nom: valeur(args, "--nom").cloned().unwrap_or_default(),
        code: valeur(args, "--code").cloned().unwrap_or_default(),
        rarete: valeur(args, "--rarete").cloned().unwrap_or_default(),
        set_nom: String::new(),
        classeur: valeur(args, "--classeur")
            .map(|c| c.to_uppercase())
            .unwrap_or_default(),
        qualite: if args.iter().any(|a| a == "--sans-etat") {
            ygo_app::inventaire::Filtre::SANS_QUALITE.to_owned()
        } else {
            valeur(args, "--qualite").cloned().unwrap_or_default()
        },
        sous_playset: args.iter().any(|a| a == "--sous-playset"),
    };

    let debut = std::time::Instant::now();
    let mut cartes = ygo_app::inventaire::lister(&paths);
    let lecture = debut.elapsed();
    ygo_app::inventaire::trier(&mut cartes, ygo_app::inventaire::Colonne::Nom, false, true);
    let vues = ygo_app::inventaire::filtrer(&cartes, &filtre, true);
    let t = ygo_app::inventaire::totaux(&vues);

    println!("Installation : {}", installation.display());
    println!(
        "{} ligne(s) possédée(s) lues en {lecture:?}{}",
        cartes.len(),
        if filtre.actif() {
            format!(" — {} après filtre", vues.len())
        } else {
            String::new()
        }
    );
    println!(
        "{} exemplaire(s) · {} playset(s) complet(s) · {} en surplus",
        t.exemplaires, t.playsets, t.surplus
    );
    println!();
    for c in vues.iter().take(200) {
        let variante = if c.variantes > 1 {
            format!(" [art {}/{}]", c.variante, c.variantes)
        } else {
            String::new()
        };
        println!(
            "  {:<14} {:<28} {:<28} ×{:<3} {:<3} {}{}",
            c.set_code,
            traquer(c.nom_affiche(true), 28),
            traquer(&c.rarete, 28),
            c.quantite,
            c.qualite,
            c.classeur,
            variante
        );
    }
    if vues.len() > 200 {
        println!("  … et {} de plus", vues.len() - 200);
    }
    Ok(true)
}

/// Tronque une chaîne à `n` caractères — pas à `n` octets, sans quoi un
/// « é » coupé en deux ferait paniquer le formatage.
fn traquer(texte: &str, n: usize) -> String {
    if texte.chars().count() <= n {
        return texte.to_owned();
    }
    texte.chars().take(n.saturating_sub(1)).collect::<String>() + "…"
}

fn cmd_etat(installation: &Path, args: &[String]) -> anyhow::Result<bool> {
    let etat = args
        .get(1)
        .filter(|a| !a.starts_with("--"))
        .context("état attendu (NM, M, EX…) — passez \"\" pour effacer")?;
    let ecrire = args.iter().any(|a| a == "--ecrire");
    let edition = valeur(args, "--edition");

    let paths = Paths::depuis_racine(installation);
    let filtre = ygo_app::inventaire::Filtre {
        nom: valeur(args, "--nom").cloned().unwrap_or_default(),
        code: valeur(args, "--code").cloned().unwrap_or_default(),
        rarete: valeur(args, "--rarete").cloned().unwrap_or_default(),
        set_nom: String::new(),
        classeur: valeur(args, "--classeur")
            .map(|c| c.to_uppercase())
            .unwrap_or_default(),
        qualite: if args.iter().any(|a| a == "--sans-etat") {
            ygo_app::inventaire::Filtre::SANS_QUALITE.to_owned()
        } else {
            String::new()
        },
        sous_playset: false,
    };

    let cartes = ygo_app::inventaire::lister(&paths);
    let vues = ygo_app::inventaire::filtrer(&cartes, &filtre, true);
    let cibles: Vec<(String, i64)> = vues.iter().map(|c| (c.classeur.clone(), c.rowid)).collect();

    let mut par_classeur: std::collections::BTreeMap<&str, usize> =
        std::collections::BTreeMap::new();
    for c in &vues {
        *par_classeur.entry(c.classeur.as_str()).or_default() += 1;
    }

    println!("Installation : {}", installation.display());
    println!(
        "Mode         : {}",
        if ecrire {
            "ÉCRITURE"
        } else {
            "analyse seule — rien ne sera modifié"
        }
    );
    println!(
        "\n{} carte(s) visée(s) sur {} possédée(s) — état « {etat} »{}",
        cibles.len(),
        cartes.len(),
        edition.map_or_else(String::new, |e| format!(", édition « {e} »"))
    );
    for (classeur, n) in &par_classeur {
        println!("  {n:5}  {classeur}");
    }

    if cibles.is_empty() {
        println!("\nRien à faire.");
        return Ok(true);
    }
    if ecrire {
        let touchees = ygo_app::inventaire::definir_qualite(&paths, &cibles, etat)?;
        println!(
            "\n{} ligne(s) mise(s) à jour dans {}",
            touchees.lignes,
            touchees.classeurs.join(", ")
        );
        if let Some(e) = edition {
            let t = ygo_app::inventaire::definir_edition(&paths, &cibles, e)?;
            println!("{} ligne(s) ont reçu l\'édition « {e} »", t.lignes);
        }
    } else {
        println!("\nRien n\'a été modifié. Relancez avec --ecrire pour appliquer.");
    }
    Ok(true)
}

fn cmd_stats(installation: &Path, args: &[String]) -> anyhow::Result<bool> {
    let detail = args.iter().any(|a| a == "--raretes");
    let seul = valeur(args, "--classeur").map(|c| c.to_uppercase());

    let paths = Paths::depuis_racine(installation);
    let debut = std::time::Instant::now();
    let tous = ygo_app::statistiques::lister(&paths);
    let lecture = debut.elapsed();
    let classeurs: Vec<&ygo_app::statistiques::Classeur> = match &seul {
        Some(code) => tous.iter().filter(|c| &c.nom == code).collect(),
        None => tous.iter().collect(),
    };
    anyhow::ensure!(!classeurs.is_empty(), "aucun classeur à montrer");

    let vus: Vec<ygo_app::statistiques::Classeur> =
        classeurs.iter().map(|c| (*c).clone()).collect();
    let t = ygo_app::statistiques::totaux(&vus);

    println!("Installation : {}", installation.display());
    println!(
        "{} classeur(s) lus en {lecture:?} — {} carte(s), {} possédée(s) ({:.1} %), \
         {} classeur(s) complet(s)",
        t.classeurs,
        t.total,
        t.possedees,
        t.pourcentage(),
        t.complets
    );
    println!();

    for c in &vus {
        let jauge = jauge(c.pourcentage());
        println!(
            "  {:<9} {jauge} {:>5.1} %   {:>4} / {:<4} — il en manque {}",
            c.nom,
            c.pourcentage(),
            c.possedees,
            c.total,
            c.manquantes()
        );
        if detail {
            for r in &c.raretes {
                println!(
                    "      {:<30} {:>4} / {:<4} ({:>5.1} %)",
                    r.rarete,
                    r.possedees,
                    r.total,
                    r.pourcentage()
                );
            }
        }
    }

    if detail {
        println!("\n  Toutes raretés confondues :");
        for r in ygo_app::statistiques::raretes_cumulees(&vus) {
            println!(
                "      {:<30} {:>4} / {:<4} ({:>5.1} %)   il en manque {}",
                r.rarete,
                r.possedees,
                r.total,
                r.pourcentage(),
                r.manquantes()
            );
        }
    }
    Ok(true)
}

/// Une jauge de vingt caractères — les polices de terminal ont `█` et `░`.
fn jauge(pourcentage: f64) -> String {
    const LARGEUR: usize = 20;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let pleines = ((pourcentage / 100.0) * LARGEUR as f64)
        .round()
        .clamp(0.0, LARGEUR as f64) as usize;
    format!("[{}{}]", "█".repeat(pleines), "░".repeat(LARGEUR - pleines))
}
