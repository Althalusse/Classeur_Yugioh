// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! L'application : l'accueil, le classeur qu'on y ouvre, et les images qui
//! arrivent pendant ce temps.
//!
//! # Un seul écrivain à la fois
//!
//! L'écran classeur détient une connexion en **écriture** sur la base du
//! classeur (règle R3). Revenir à l'accueil détruit cet écran, donc referme la
//! connexion — c'est ce qui autorise à rouvrir ensuite n'importe quel autre
//! classeur, et ce qui garantit qu'aucune base n'a deux écrivains.
//!
//! L'accueil, lui, survit à l'aller-retour. Il est simplement **rechargé** au
//! retour : les quantités ont pu changer, les couvertures non.
//!
//! # Les téléchargements ne passent pas par les écrans
//!
//! Le service vit ici, au-dessus des deux. Les écrans ne le connaissent pas :
//! l'un reçoit un ordre de rafraîchir son cache, l'autre rien du tout. C'est
//! ce qui les garde dessinables sans réseau, et testables sans fenêtre.

use std::path::PathBuf;

use eframe::egui;
use ygo_ui::accueil::EcranAccueil;
use ygo_ui::artworks::{EcranArtworks, Portee};
use ygo_ui::classeur::EcranClasseur;
use ygo_ui::corbeille::EcranCorbeille;
use ygo_ui::initialisation::EcranInitialisation;
use ygo_ui::inventaire::EcranInventaire;
use ygo_ui::options::EcranOptions;
use ygo_ui::selecteur::EcranSelecteur;
use ygo_ui::statistiques::EcranStatistiques;
use ygo_ui::telechargements::Service;

fn main() -> eframe::Result<()> {
    let mut args = std::env::args().skip(1);
    // Sans argument, la racine est le dossier de l'exécutable.
    //
    // C'est la convention du Python (`centralisation_dossier.get_exe_dir()`),
    // et c'est ce qui fait la différence entre un binaire de laboratoire et
    // une application : posé dans son dossier, `ygo-ui.exe` trouve ses données
    // à côté de lui, sans lanceur ni chemin à connaître. L'argument reste
    // prioritaire — c'est ce dont le développement se sert, puisque
    // `cargo run` place l'exécutable dans `target/debug`.
    let racine = match args.next() {
        Some(chemin) => PathBuf::from(chemin),
        None => match ygo_core::paths::Paths::depuis_executable() {
            Ok(paths) => paths.racine().to_path_buf(),
            Err(e) => {
                eprintln!(
                    "impossible de déterminer le dossier de l'application : {e}\n\
                     \n\
                     usage : ygo-ui [installation] [CODE]\n\
                     Sans argument, les données sont cherchées à côté de l'exécutable."
                );
                std::process::exit(2);
            }
        },
    };
    let direct = args.next();

    // La journalisation, avant tout le reste.
    //
    // `ygo_core::log` existait, testé, depuis le premier jour — et **personne
    // ne l'appelait**. Chaque `tracing::warn!` du projet partait donc dans le
    // vide : une couverture non téléchargée, une passe artworks en échec, une
    // mise à jour interrompue ne laissaient aucune trace. Lancée par
    // `Lancer-Appli.ps1`, la fenêtre n'a même pas de console où le voir.
    //
    // La garde est tenue jusqu'à la fin de `main` : la lâcher plus tôt ferme
    // le fichier et perd les dernières lignes — exactement celles qui
    // expliquent un arrêt.
    let paths = ygo_core::paths::Paths::depuis_racine(&racine);
    let _journal = match ygo_core::log::installer(paths.logs()) {
        Ok(garde) => {
            ygo_core::log::entete_session(env!("CARGO_PKG_VERSION"));
            tracing::info!(installation = %racine.display(), "interface démarrée");
            Some(garde)
        }
        Err(e) => {
            // Sans journal on continue : c'est un outil de diagnostic, pas une
            // dépendance de l'application.
            eprintln!("journalisation indisponible : {e}");
            None
        }
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 900.0])
            .with_title("Yu-Gi-Oh! Collection Manager"),
        ..Default::default()
    };
    eframe::run_native(
        "ygo-ui",
        options,
        Box::new(move |cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            // Avant tout dessin : la base porte 14 205 textes japonais, et
            // aucune des quatre polices d'egui n'a d'idéogramme.
            ygo_ui::polices::installer(&cc.egui_ctx);
            let mut application = Application::nouvelle(racine, cc.egui_ctx.clone());
            if let Some(code) = direct {
                application.ouvrir_classeur(&code);
            }
            Ok(Box::new(application))
        }),
    )
}

/// L'écran affiché.
enum Ecran {
    Accueil,
    /// Le premier lancement : pas encore de `cardinfo.db`.
    Initialisation(Box<EcranInitialisation>),
    Classeur(Box<EcranClasseur>),
    Options(Box<EcranOptions>),
    Selecteur(Box<EcranSelecteur>),
    Corbeille(Box<EcranCorbeille>),
    Artworks(Box<EcranArtworks>),
    Inventaire(Box<EcranInventaire>),
    Statistiques(Box<EcranStatistiques>),
}

struct Application {
    accueil: EcranAccueil,
    ecran: Ecran,
    /// Le fil de téléchargement et ce qu'il rapporte.
    images: Service,
    /// Ce qui a empêché d'ouvrir le dernier classeur demandé.
    erreur: Option<String>,
    /// Le classeur à rouvrir au retour des options, s'il y en avait un.
    ///
    /// Changer la langue ou l'ordre de tri change ce que l'écran classeur
    /// **calcule** : le rouvrir est plus sûr que de tenter de le recalculer en
    /// place, et c'est instantané — six millisecondes de lecture.
    retour_vers: Option<String>,
}

impl Application {
    fn nouvelle(racine: PathBuf, ctx: egui::Context) -> Self {
        let images = Service::demarrer(racine.clone(), ctx);
        let accueil = EcranAccueil::ouvrir(racine);
        // Sans base, l'accueil est un écran vide qui n'explique rien. Le
        // premier lancement mérite mieux — et c'est ce que le Python faisait
        // depuis toujours (PHASE 1 de `main.py`), avant même de construire sa
        // fenêtre principale.
        let ecran = if EcranInitialisation::necessaire(accueil.paths()) {
            Ecran::Initialisation(Box::new(EcranInitialisation::ouvrir(
                accueil.paths().clone(),
            )))
        } else {
            Ecran::Accueil
        };
        Self {
            accueil,
            ecran,
            images,
            erreur: None,
            retour_vers: None,
        }
    }

    /// Ouvre un classeur, ou retient pourquoi il n'a pas pu l'être.
    ///
    /// Un classeur illisible ne fait pas tomber l'application : elle reste sur
    /// l'accueil et le dit. C'est le cas d'une base verrouillée par une autre
    /// instance, ou d'un dossier de classeur sans `.db`.
    fn ouvrir_classeur(&mut self, code: &str) {
        match EcranClasseur::ouvrir(self.accueil.racine().clone(), code) {
            Ok(ecran) => {
                self.erreur = None;
                self.ecran = Ecran::Classeur(Box::new(ecran));
                // L'ouverture déclenche la vérification des images. Le service
                // ne va sur le réseau que s'il manque quelque chose, et ne
                // redemande pas un classeur déjà vu dans cette session.
                self.images.completer(code);
            }
            Err(e) => {
                self.erreur = Some(format!("Impossible d'ouvrir {code} : {e}"));
                self.ecran = Ecran::Accueil;
            }
        }
    }

    /// Retire un classeur, du mode que l'utilisateur a choisi.
    ///
    /// L'écran d'accueil ne le fait pas lui-même : il demande. C'est la même
    /// séparation que partout ailleurs — l'écran dit ce que l'utilisateur veut,
    /// l'application décide et agit.
    ///
    /// Une suppression définitive est **journalisée en `warn`** : c'est le seul
    /// geste de l'application qui détruise des données de l'utilisateur sans
    /// recours, et il doit rester une trace de l'avoir fait.
    fn supprimer_classeur(&mut self, code: &str, mode: ygo_app::suppression::Mode) {
        let paths = self.accueil.paths().clone();
        let horodatage = ygo_app::suppression::horodatage_maintenant();
        match ygo_app::suppression::supprimer(&paths, code, mode, &horodatage) {
            Ok(ou) => {
                if mode.est_sans_retour() {
                    tracing::warn!(classeur = %code, dossier = %ou.display(), "classeur effacé définitivement");
                    println!("{code} — effacé définitivement : {}", ou.display());
                } else {
                    tracing::info!(classeur = %code, dossier = %ou.display(), "classeur mis à la corbeille");
                    println!("{code} — mis à la corbeille : {}", ou.display());
                }
                self.erreur = None;
                self.accueil.recharger_tout();
            }
            Err(e) => self.erreur = Some(format!("{code} — suppression impossible : {e}")),
        }
    }

    /// Ouvre l'écran de création.
    fn ouvrir_selecteur(&mut self, ctx: &egui::Context) {
        self.ecran = Ecran::Selecteur(Box::new(EcranSelecteur::ouvrir(
            self.accueil.racine().clone(),
            ctx.clone(),
        )));
    }

    /// Ouvre la corbeille.
    fn ouvrir_corbeille(&mut self) {
        self.ecran = Ecran::Corbeille(Box::new(EcranCorbeille::ouvrir(
            self.accueil.paths().clone(),
        )));
    }

    /// Ouvre l'écran des artworks, sur la portée demandée.
    fn ouvrir_artworks(&mut self, portee: Portee) {
        self.ecran = Ecran::Artworks(Box::new(EcranArtworks::ouvrir(
            self.accueil.paths().clone(),
            portee,
        )));
    }

    /// Ouvre l'inventaire.
    ///
    /// La langue d'affichage vient des réglages, comme partout ailleurs :
    /// l'inventaire montre les mêmes noms que le classeur.
    fn ouvrir_inventaire(&mut self) {
        let francais = ygo_core::config::Config::charger(self.accueil.paths().app_config())
            .langue()
            .code()
            == "FR";
        self.ecran = Ecran::Inventaire(Box::new(EcranInventaire::ouvrir(
            self.accueil.paths().clone(),
            francais,
        )));
    }

    /// Ouvre les statistiques.
    fn ouvrir_stats(&mut self) {
        self.ecran = Ecran::Statistiques(Box::new(EcranStatistiques::ouvrir(
            self.accueil.paths().clone(),
        )));
    }

    /// Ouvre les options, en retenant d'où l'on vient.
    fn ouvrir_options(&mut self) {
        self.retour_vers = match &self.ecran {
            Ecran::Classeur(c) => Some(c.code().to_owned()),
            Ecran::Accueil
            | Ecran::Initialisation(_)
            | Ecran::Options(_)
            | Ecran::Selecteur(_)
            | Ecran::Corbeille(_)
            | Ecran::Artworks(_)
            | Ecran::Inventaire(_)
            | Ecran::Statistiques(_) => None,
        };
        self.ecran = Ecran::Options(Box::new(EcranOptions::ouvrir(self.accueil.paths().clone())));
    }

    /// Applique ce qu'un changement de réglage oblige à refaire.
    ///
    /// L'écran des options ne fait rien lui-même : il dit ce qu'il a touché.
    /// C'est ici qu'on décide, parce que c'est ici qu'on sait quel écran est
    /// ouvert.
    fn appliquer(&mut self, consequences: ygo_ui::options::Consequences) {
        if consequences.relire_accueil || consequences.relire_classeur {
            self.accueil.recharger();
        }
        match self.retour_vers.take() {
            // Rouvrir le classeur relit ses cartes avec les nouveaux réglages
            // et repart d'un cache d'images vide — ce qui règle du même coup
            // le changement de source. Six millisecondes de lecture : ça ne
            // vaut pas la peine de distinguer les cas.
            Some(code) => self.ouvrir_classeur(&code),
            None => self.ecran = Ecran::Accueil,
        }
    }

    /// Vide la file d'événements et en tire les conséquences.
    fn traiter_images(&mut self) {
        let recus = self.images.recevoir();
        if recus.is_empty() {
            return;
        }
        // Les images arrivées rendent leur chance aux cartes qui montraient un
        // emplacement vide — mais seulement si c'est le classeur ouvert.
        let a_rafraichir = self.images.etat_mut().a_rafraichir();
        if let Ecran::Classeur(classeur) = &mut self.ecran {
            if a_rafraichir.iter().any(|code| code == classeur.code()) {
                classeur.rafraichir_images();
            }
        }
        // La base construite fait sortir du premier lancement : l'écran a fini
        // sa raison d'être, et l'accueil est enfin utile.
        if matches!(self.ecran, Ecran::Initialisation(_))
            && !EcranInitialisation::necessaire(self.accueil.paths())
            && !self.images.base_en_cours()
        {
            self.ecran = Ecran::Accueil;
        }
        // Un classeur créé doit apparaître à l'accueil sans que l'utilisateur
        // ait à faire quoi que ce soit.
        if !self.images.etat_mut().creations().is_empty() {
            // `recharger_tout` et non `recharger` : un classeur neuf a une
            // couverture que l'accueil n'a jamais lue.
            self.accueil.recharger_tout();
        }
    }

    /// Le bandeau « mise à jour disponible ».
    ///
    /// # Pourquoi un bandeau et pas un bouton de plus
    ///
    /// La barre du haut de l'accueil porte déjà huit contrôles. Un neuvième,
    /// même coloré, se serait fondu dans la rangée — et n'aurait rien annoncé
    /// depuis l'écran classeur, où l'on passe le plus clair du temps. Un
    /// bandeau traverse la fenêtre, se voit depuis n'importe quel écran, et
    /// porte son action : un clic met à jour, sans passer par les Options.
    ///
    /// # Ce qu'il dit
    ///
    /// Les deux versions, pas seulement « une mise à jour est disponible ».
    /// Savoir qu'on passe de `146.68` à `147.01` permet de juger ; un bandeau
    /// qui n'annonce que son existence ne se lit qu'une fois.
    ///
    /// Il s'écarte, et ne revient pas de la session. Il **reparaît** au
    /// démarrage suivant : une base périmée le reste, et un bandeau qu'on ne
    /// peut faire taire que pour de bon finit par être fermé sans être lu.
    fn bandeau_maj(&mut self, ui: &mut egui::Ui) {
        if !self.images.etat().maj_disponible() {
            return;
        }
        let Some(ygo_app::maj::Verdict::AFaire { locale, distante }) =
            self.images.etat().version().cloned()
        else {
            return;
        };

        let mut lancer = false;
        let mut ecarter = false;
        egui::Panel::top("bandeau-maj").show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let accent = ui.visuals().warn_fg_color;
                ui.colored_label(
                    accent,
                    egui::RichText::new("⬆ Mise à jour de la base disponible").strong(),
                );
                ui.add_space(6.0);
                ui.label(match &locale {
                    Some(v) => format!("version {distante} — vous avez la {v}"),
                    None => format!("version {distante} — aucune version enregistrée ici"),
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // ✖ (U+2716) : cf. la note du bandeau d'erreur.
                    if ui
                        .button("✖")
                        .on_hover_text("Ne plus le montrer jusqu'au prochain démarrage")
                        .clicked()
                    {
                        ecarter = true;
                    }
                    ui.add_space(8.0);
                    if ui
                        .button("⟳ Mettre à jour")
                        .on_hover_text(
                            "Environ 35 Mo, moins d'une minute — vos classeurs \
                             ne sont pas touchés",
                        )
                        .clicked()
                    {
                        lancer = true;
                    }
                });
            });
            ui.add_space(6.0);
        });

        if lancer {
            self.images.mettre_a_jour_base();
        }
        if ecarter {
            self.images.ecarter_maj();
        }
    }

    /// La bande de progression, au-dessus des écrans.
    ///
    /// # Ce que la première version faisait mal
    ///
    /// Le libellé à gauche, la barre à l'extrême droite, un mètre de vide
    /// entre les deux : l'œil ne rapprochait jamais les deux moitiés d'une
    /// même information. La barre était fine, sans texte, et l'étape en cours
    /// n'était pas dite — « Images » s'affichait même pendant la création.
    ///
    /// Désormais : un seul groupe centré, une barre haute qui **porte son
    /// propre texte**, et l'étape nommée. Une tâche dont le total est encore
    /// inconnu — création, artworks — montre une barre animée plutôt qu'une
    /// progression inventée.
    fn bande_images(&mut self, ui: &mut egui::Ui) {
        let etat = self.images.etat();
        if !etat.actif() {
            return;
        }
        let cumul = etat.cumul();
        let classeurs: Vec<String> = etat.en_cours().keys().cloned().collect();
        let etape = classeurs
            .first()
            .and_then(|code| etat.etape(code))
            .map_or("Travail", ygo_ui::telechargements::Etape::libelle);
        let quoi = classeurs.join(", ");

        // Le rang dans le lot, à gauche de la barre.
        //
        // Le fil traite une commande à la fois : trois classeurs demandés d'un
        // coup ne paraissaient qu'un par un, et rien ne disait que deux autres
        // attendaient. « 1/3 » le dit en quatre caractères.
        let lot = etat.lot();
        let attendent: Vec<String> = etat.en_attente().iter().map(|c| (*c).to_owned()).collect();

        egui::Panel::top("telechargements").show(ui, |ui| {
            ui.add_space(8.0);
            ui.vertical_centered(|ui| {
                let barre = if cumul.total == 0 {
                    // Total inconnu : une barre animée dit « ça travaille »
                    // sans prétendre savoir où l'on en est.
                    egui::ProgressBar::new(0.0)
                        .animate(true)
                        .text(format!("{etape} — {quoi}"))
                } else {
                    #[allow(clippy::cast_precision_loss)]
                    let part = cumul.faites as f32 / cumul.total as f32;
                    // La base compte en octets, les classeurs en images : le
                    // même nombre ne se lit pas de la même façon. « 45 / 162 Mo »
                    // se comprend ; « 47185920 / 169869312 » ne se lit pas.
                    let (faites, total, unite) = if quoi == ygo_ui::telechargements::CODE_BASE {
                        (cumul.faites / 1_048_576, cumul.total / 1_048_576, " Mo")
                    } else {
                        (cumul.faites, cumul.total, "")
                    };
                    egui::ProgressBar::new(part).text(format!(
                        "{etape} — {quoi}   {faites} / {total}{unite}   ({:.0} %)",
                        part * 100.0
                    ))
                };
                ui.horizontal(|ui| {
                    if let Some((rang, total)) = lot {
                        let etiquette = ui.label(
                            egui::RichText::new(format!("{rang}/{total}"))
                                .strong()
                                .monospace(),
                        );
                        if !attendent.is_empty() {
                            etiquette
                                .on_hover_text(format!("en attente : {}", attendent.join(", ")));
                        }
                        ui.add_space(8.0);
                    }
                    ui.add_sized([560.0, 22.0], barre);
                });
            });
            ui.add_space(8.0);
        });
    }
}

impl eframe::App for Application {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.traiter_images();

        // Le message d'erreur d'ouverture, au-dessus de tout le reste.
        if let Some(message) = self.erreur.clone() {
            egui::Panel::top("erreur-ouverture").show(ui, |ui| {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.colored_label(ui.visuals().error_fg_color, message.clone());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // ✖ (U+2716) et non ✕ (U+2715) : le second n'est
                        // dans aucune des trois polices d'egui, et ne
                        // s'affichait qu'en carré vide.
                        if ui.button("✖").clicked() {
                            self.erreur = None;
                        }
                    });
                });
                ui.add_space(6.0);
            });
        }

        // Ni bandeau ni bande pendant le premier lancement : l'écran porte
        // déjà son propre avancement, et « mise à jour disponible » y serait
        // une évidence bruyante — il n'y a pas de base du tout.
        if !matches!(self.ecran, Ecran::Initialisation(_)) {
            self.bandeau_maj(ui);
            self.bande_images(ui);
        }

        match &mut self.ecran {
            Ecran::Initialisation(init) => {
                init.signaler(self.images.etat());
                eframe::App::ui(init.as_mut(), ui, frame);
                if init.construction_demandee() {
                    self.images.mettre_a_jour_base();
                }
                if init.passer_outre() {
                    self.ecran = Ecran::Accueil;
                }
            }
            Ecran::Accueil => {
                eframe::App::ui(&mut self.accueil, ui, frame);
                if let Some(code) = self.accueil.demande_ouverture() {
                    self.ouvrir_classeur(&code);
                }
                if self.accueil.demande_options() {
                    self.ouvrir_options();
                }
                if self.accueil.demande_creation() {
                    self.ouvrir_selecteur(ui.ctx());
                }
                if let Some((code, mode)) = self.accueil.suppression_confirmee() {
                    self.supprimer_classeur(&code, mode);
                }
                if self.accueil.demande_corbeille() {
                    self.ouvrir_corbeille();
                }
                if let Some(classeur) = self.accueil.demande_artworks() {
                    self.ouvrir_artworks(match classeur {
                        Some(code) => Portee::Classeur(code),
                        None => Portee::Tout,
                    });
                }
                if self.accueil.demande_inventaire() {
                    self.ouvrir_inventaire();
                }
                // Les classeurs que l'import réclame partent sur le fil de
                // travail, exactement comme depuis l'inventaire.
                for code in self.accueil.creations_csv_demandees() {
                    self.images.creer(&code, true);
                }
                if self.accueil.demande_stats() {
                    self.ouvrir_stats();
                }
            }
            Ecran::Classeur(classeur) => {
                eframe::App::ui(classeur.as_mut(), ui, frame);
                if classeur.options_demandees() {
                    self.ouvrir_options();
                } else if let Some(set_code) = classeur.artworks_demandes() {
                    // Détruire l'écran referme sa connexion en écriture :
                    // l'écran des artworks écrit dans la même base, et deux
                    // écrivains sur un classeur, jamais (règle R3).
                    let code = classeur.code().to_owned();
                    self.ouvrir_artworks(Portee::Carte {
                        classeur: code,
                        set_code,
                    });
                } else if classeur.retour_demande() {
                    // Détruire l'écran referme sa connexion en écriture.
                    self.ecran = Ecran::Accueil;
                    self.accueil.recharger();
                }
            }
            Ecran::Options(options) => {
                // L'état de la mise à jour est poussé AVANT le rendu : le
                // bouton doit être grisé dès la frame où le fil travaille,
                // pas à la suivante.
                options.signaler_maj(self.images.base_en_cours(), self.images.etat().version());
                eframe::App::ui(options.as_mut(), ui, frame);
                if options.maj_demandee() {
                    self.images.mettre_a_jour_base();
                }
                if options.verification_demandee() {
                    self.images.verifier_version();
                }
                let consequences = options.consequences();
                if options.retour_demande() {
                    self.appliquer(consequences);
                }
            }
            Ecran::Corbeille(corbeille) => {
                eframe::App::ui(corbeille.as_mut(), ui, frame);
                // Une restauration remet un dossier de classeur en place :
                // l'accueil doit le relire, couverture comprise, car il ne
                // l'a peut-être jamais vu. Le compte de la corbeille suit
                // dans la foulée — c'est `recharger` qui le recalcule.
                let restauration = corbeille.restauration_faite();
                if corbeille.retour_demande() {
                    self.ecran = Ecran::Accueil;
                    self.accueil.recharger_tout();
                } else if restauration {
                    self.accueil.recharger_tout();
                }
            }
            Ecran::Artworks(artworks) => {
                eframe::App::ui(artworks.as_mut(), ui, frame);
                // Les aperçus partent sur le fil de téléchargement, comme
                // les images d'un classeur : l'écran demande, l'application
                // fait. Le classeur nommé ne sert qu'à libeller l'avancement.
                let cibles = artworks.apercus_demandes();
                if !cibles.is_empty() {
                    let etiquette = artworks
                        .portee()
                        .classeur()
                        .unwrap_or("artworks")
                        .to_owned();
                    self.images.apercus(&etiquette, cibles);
                }
                // Poser un artwork ajoute une ligne au classeur : le
                // rouvrir est plus sûr que de deviner ce qui a bougé.
                let ecrit = artworks.a_ecrit();
                let classeur = artworks.portee().classeur().map(ToOwned::to_owned);
                if artworks.retour_demande() {
                    match classeur {
                        Some(code) => self.ouvrir_classeur(&code),
                        None => {
                            self.ecran = Ecran::Accueil;
                            self.accueil.recharger();
                        }
                    }
                } else if ecrit {
                    self.accueil.recharger();
                }
            }
            Ecran::Inventaire(inventaire) => {
                eframe::App::ui(inventaire.as_mut(), ui, frame);
                // La création part sur le fil de travail, comme celle du
                // sélecteur : l'écran demande, l'application fait.
                for code in inventaire.creations_demandees() {
                    self.images.creer(&code, true);
                }
                if inventaire.retour_demande() {
                    // L'inventaire écrit des quantités : l'accueil doit
                    // relire ses compteurs. Les couvertures, elles, n'ont
                    // pas bougé.
                    self.ecran = Ecran::Accueil;
                    self.accueil.recharger();
                }
            }
            Ecran::Statistiques(stats) => {
                eframe::App::ui(stats.as_mut(), ui, frame);
                if stats.retour_demande() {
                    // Les statistiques ne modifient rien : l'accueil n'a
                    // aucune raison de se relire au retour.
                    self.ecran = Ecran::Accueil;
                }
            }
            Ecran::Selecteur(selecteur) => {
                eframe::App::ui(selecteur.as_mut(), ui, frame);
                let avec_artworks = selecteur.avec_artworks();
                if let Some(code) = selecteur.creation_demandee() {
                    // On RESTE sur le sélecteur.
                    //
                    // La première version revenait à l'accueil, au motif que
                    // rester donnerait l'impression que rien ne se passe.
                    // Retour d'usage du 2026-09-05 : c'est l'inverse qui gêne.
                    // On vient ici pour créer, souvent plusieurs sets à la
                    // suite, et chaque retour forcé coûtait un aller-retour
                    // plus une relecture des 2 385 sets — visible trois fois
                    // dans le journal du même soir.
                    //
                    // L'objection de départ tenait à une absence d'accusé de
                    // réception, pas au fait de rester : l'écran marque
                    // désormais les sets lancés et les rappelle en bandeau.
                    self.images.creer(&code, avec_artworks);
                } else if selecteur.retour_demande() {
                    self.ecran = Ecran::Accueil;
                }
            }
        }

        // Le titre de la fenêtre suit l'écran — c'est ce que la barre des
        // tâches de Windows montre.
        let titre = match &self.ecran {
            Ecran::Accueil | Ecran::Initialisation(_) => "Yu-Gi-Oh! Collection Manager".to_owned(),
            Ecran::Classeur(c) => format!("{} — Yu-Gi-Oh! Collection Manager", c.code()),
            Ecran::Options(_) => "Options — Yu-Gi-Oh! Collection Manager".to_owned(),
            Ecran::Selecteur(_) => "Nouveau classeur — Yu-Gi-Oh! Collection Manager".to_owned(),
            Ecran::Corbeille(_) => "Corbeille — Yu-Gi-Oh! Collection Manager".to_owned(),
            Ecran::Artworks(_) => "Artworks — Yu-Gi-Oh! Collection Manager".to_owned(),
            Ecran::Inventaire(_) => "Toutes mes cartes — Yu-Gi-Oh! Collection Manager".to_owned(),
            Ecran::Statistiques(_) => "Statistiques — Yu-Gi-Oh! Collection Manager".to_owned(),
        };
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::Title(titre));
    }
}
