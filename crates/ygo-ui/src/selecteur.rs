// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! L'écran de création d'un classeur : choisir un set.
//!
//! Portage de `ui/ecran_selecteur_set.py`. La matière vient de
//! [`ygo_app::selecteur`], éprouvé sans fenêtre.
//!
//! # La liste se charge sur un fil, et ce n'est pas de la précaution
//!
//! Mesuré sur la `cardinfo.db` réelle — 163 Mo, 1 507 sets — les deux requêtes
//! d'agrégat prennent **708 ms**. Les tenir dans la boucle de rendu figerait la
//! fenêtre presque une seconde à chaque ouverture de l'écran. Elle est donc
//! lue une fois, sur un fil, et l'écran affiche son attente.
//!
//! # L'écran part vide, à dessein
//!
//! `ecran_selecteur_set.py` documente pourquoi : afficher mille cinq cents
//! sets d'un coup ne rend service à personne et coûte le prix de mille cinq
//! cents widgets. En dessous de deux caractères, la liste reste vide et le
//! dit — ce n'est pas « aucun résultat ».

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};

use eframe::egui;
use ygo_app::selecteur::{self, SetDisponible, MAX_RESULTATS, MIN_RECHERCHE};
use ygo_core::config::Config;
use ygo_core::paths::Paths;

/// L'état du chargement de la liste.
enum Chargement {
    /// Le fil travaille.
    EnCours(Receiver<Vec<SetDisponible>>),
    /// La liste est là.
    Prete(Vec<SetDisponible>),
}

/// L'écran de sélection d'un set.
pub struct EcranSelecteur {
    chargement: Chargement,
    terme: String,
    focus: bool,

    /// Le set qu'un clic demande à créer.
    creer: Option<String>,
    /// Les sets lancés depuis l'ouverture de cet écran.
    ///
    /// L'écran ne se referme plus après une création (retour d'usage du
    /// 2026-09-05 : « il faudrait rester sur la page de création si
    /// l'utilisateur veut créer plusieurs classeurs d'un coup »). Il faut donc
    /// qu'il montre ce qu'il a déjà lancé — sans quoi, la liste étant
    /// inchangée, rien ne distinguerait un clic pris en compte d'un clic
    /// perdu.
    lances: Vec<String>,
    /// Un retour a été demandé.
    retour: bool,
    /// Les artworks Yugipedia seront-ils complétés à la création ?
    avec_artworks: bool,
}

impl std::fmt::Debug for EcranSelecteur {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EcranSelecteur")
            .field("terme", &self.terme)
            .finish_non_exhaustive()
    }
}

impl EcranSelecteur {
    /// Ouvre l'écran et lance la lecture de la liste en arrière-plan.
    #[must_use]
    pub fn ouvrir(racine: PathBuf, ctx: egui::Context) -> Self {
        let paths = Paths::depuis_racine(&racine);
        let config = Config::charger(paths.app_config());
        let francais = config.langue().code() == "FR";
        let avec_artworks = config.completer_artworks_yugipedia();

        let (envoi, reception) = std::sync::mpsc::channel();
        let lecture = std::thread::Builder::new()
            .name("ygo-sets".to_owned())
            .spawn(move || {
                let debut = std::time::Instant::now();
                let sets = selecteur::lister(&paths, francais).unwrap_or_default();
                println!(
                    "sélecteur — {} sets lus en {:?}",
                    sets.len(),
                    debut.elapsed()
                );
                let _ = envoi.send(sets);
                ctx.request_repaint();
            });
        if let Err(e) = lecture {
            tracing::error!(erreur = %e, "lecture des sets non démarrée");
        }

        Self {
            chargement: Chargement::EnCours(reception),
            terme: String::new(),
            focus: true,
            creer: None,
            lances: Vec::new(),
            retour: false,
            avec_artworks,
        }
    }

    /// Reprend la demande de création.
    pub fn creation_demandee(&mut self) -> Option<String> {
        self.creer.take()
    }

    /// Reprend la demande de retour.
    pub fn retour_demande(&mut self) -> bool {
        std::mem::take(&mut self.retour)
    }

    /// Les artworks doivent-ils être complétés à la création ?
    #[must_use]
    pub fn avec_artworks(&self) -> bool {
        self.avec_artworks
    }

    /// Récupère la liste si le fil l'a rendue.
    fn recevoir(&mut self) {
        if let Chargement::EnCours(reception) = &self.chargement {
            match reception.try_recv() {
                Ok(sets) => self.chargement = Chargement::Prete(sets),
                Err(TryRecvError::Empty) => {}
                // Le fil est mort sans rien envoyer : liste vide plutôt
                // qu'attente éternelle.
                Err(TryRecvError::Disconnected) => {
                    self.chargement = Chargement::Prete(Vec::new());
                }
            }
        }
    }
}

impl eframe::App for EcranSelecteur {
    fn ui(&mut self, racine: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.recevoir();
        self.barre_du_haut(racine);
        self.bandeau_lances(racine);
        egui::CentralPanel::default().show(racine, |ui| {
            let Chargement::Prete(sets) = &self.chargement else {
                ui.vertical_centered(|ui| {
                    ui.add_space(96.0);
                    ui.spinner();
                    ui.add_space(8.0);
                    ui.label("Lecture des sets…");
                    ui.small("cardinfo.db déclare plus de mille cinq cents sets");
                });
                return;
            };

            if sets.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(96.0);
                    ui.heading("Aucun set disponible.");
                    ui.label(
                        "cardinfo.db est absente ou n'a pas été initialisée.",
                    );
                    ui.add_space(4.0);
                    ui.small(
                        "Options ▸ Base de référence ▸ Construire la base — \
                         environ 35 Mo, moins d'une minute.",
                    );
                });
                return;
            }

            let resultats = selecteur::filtrer(sets, &self.terme, MAX_RESULTATS);
            if resultats.trop_court {
                ui.vertical_centered(|ui| {
                    ui.add_space(64.0);
                    ui.label(format!(
                        "Tapez au moins {MIN_RECHERCHE} caractères — code ou nom du set."
                    ));
                    ui.add_space(4.0);
                    ui.small(format!("{} sets connus", sets.len()));
                });
                return;
            }

            if resultats.sets.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(64.0);
                    ui.heading(format!("Aucun set ne correspond à « {} ».", self.terme.trim()));
                });
                return;
            }

            let mut demande: Option<String> = None;
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(8.0);
                if resultats.tronquee() {
                    ui.label(format!(
                        "{} correspondances — les {} premières sont montrées. Affinez la recherche.",
                        resultats.total,
                        resultats.sets.len()
                    ));
                    ui.add_space(8.0);
                }
                for set in &resultats.sets {
                    if carte_set(ui, set, self.lances.contains(&set.code)) {
                        demande = Some(set.code.clone());
                    }
                }
            });
            if let Some(code) = demande {
                if !self.lances.contains(&code) {
                    self.lances.push(code.clone());
                }
                self.creer = Some(code);
            }
        });
    }
}

impl EcranSelecteur {
    /// Ce que l'écran a déjà lancé, et ce qu'il en advient.
    ///
    /// Sans cette ligne, rester sur l'écran après une création serait pire que
    /// le quitter : la liste ne bouge pas, le classeur naît ailleurs, et rien
    /// ne dirait que le clic a porté.
    fn bandeau_lances(&self, racine: &mut egui::Ui) {
        if self.lances.is_empty() {
            return;
        }
        egui::Panel::top("lances-selecteur").show(racine, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!(
                    "✔ {} classeur(s) lancé(s)",
                    self.lances.len()
                )));
                ui.add_space(8.0);
                ui.small(self.lances.join(", "));
                ui.add_space(12.0);
                ui.small(
                    "ils se créent en arrière-plan — l'avancement est en haut, \
                     et ils apparaissent à l'accueil au fur et à mesure",
                );
            });
            ui.add_space(6.0);
        });
    }

    fn barre_du_haut(&mut self, racine: &mut egui::Ui) {
        egui::Panel::top("entete-selecteur").show(racine, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("◀ Mes classeurs").clicked() {
                    self.retour = true;
                }
                ui.add_space(8.0);
                ui.heading("Nouveau classeur");
                ui.add_space(16.0);
                let champ = ui.add(
                    egui::TextEdit::singleline(&mut self.terme)
                        .hint_text("code ou nom du set…")
                        .desired_width(260.0),
                );
                // Le curseur est dans le champ dès l'ouverture : on vient ici
                // pour chercher, pas pour cliquer dans une case d'abord.
                if std::mem::take(&mut self.focus) {
                    champ.request_focus();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.checkbox(&mut self.avec_artworks, "artworks Yugipedia")
                        .on_hover_text(
                            "Complète les variantes d'illustration après la création. \
                             Demande le réseau et allonge la création.",
                        );
                });
            });
            ui.add_space(8.0);
        });
    }
}

/// Une carte de set. Rend `true` si la création est demandée.
fn carte_set(ui: &mut egui::Ui, set: &SetDisponible, lance: bool) -> bool {
    let mut demande = false;
    egui::Frame::group(ui.style())
        .corner_radius(6.0)
        .inner_margin(10.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width() - 24.0);
            ui.horizontal(|ui| {
                ui.add_sized(
                    [110.0, 20.0],
                    egui::Label::new(egui::RichText::new(set.code.as_str()).strong().monospace()),
                );
                ui.add_space(8.0);
                ui.add(egui::Label::new(set.nom.as_str()).truncate());

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if set.deja_cree {
                        ui.label(egui::RichText::new("déjà créé").weak());
                    } else if lance {
                        // La liste des sets n'est pas relue à chaque création
                        // — `deja_cree` resterait donc faux, et le bouton
                        // « Créer » réapparaîtrait sur un set déjà lancé.
                        ui.label(egui::RichText::new("✔ lancé").weak());
                    } else if ui.button("Créer").clicked() {
                        demande = true;
                    }
                    ui.add_space(12.0);
                    ui.small(format!("{} cartes", set.nb_cartes));
                });
            });
        });
    ui.add_space(4.0);
    demande
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    /// Le fil qui meurt sans rien envoyer ne doit pas laisser l'écran en
    /// attente pour toujours : il bascule sur une liste vide, qui a son propre
    /// message.
    #[test]
    fn un_fil_mort_ne_laisse_pas_l_ecran_en_attente() {
        let (envoi, reception) = std::sync::mpsc::channel::<Vec<SetDisponible>>();
        let mut ecran = EcranSelecteur {
            chargement: Chargement::EnCours(reception),
            terme: String::new(),
            focus: false,
            creer: None,
            lances: Vec::new(),
            retour: false,
            avec_artworks: true,
        };
        assert!(matches!(ecran.chargement, Chargement::EnCours(_)));

        drop(envoi);
        ecran.recevoir();
        match &ecran.chargement {
            Chargement::Prete(sets) => assert!(sets.is_empty()),
            Chargement::EnCours(_) => panic!("l'écran attend encore"),
        }
    }

    /// Tant que rien n'arrive, l'écran reste en attente — il ne conclut pas à
    /// une liste vide au premier coup d'œil.
    #[test]
    fn sans_reponse_l_ecran_reste_en_attente() {
        let (_envoi, reception) = std::sync::mpsc::channel::<Vec<SetDisponible>>();
        let mut ecran = EcranSelecteur {
            chargement: Chargement::EnCours(reception),
            terme: String::new(),
            focus: false,
            creer: None,
            lances: Vec::new(),
            retour: false,
            avec_artworks: true,
        };
        ecran.recevoir();
        assert!(matches!(ecran.chargement, Chargement::EnCours(_)));
    }

    /// Rester sur l'écran après une création n'aurait aucun sens s'il ne
    /// montrait pas ce qu'il a lancé : la liste des sets, elle, ne bouge pas.
    #[test]
    fn les_sets_lances_sont_retenus() {
        let (_envoi, reception) = std::sync::mpsc::channel::<Vec<SetDisponible>>();
        let mut ecran = EcranSelecteur {
            chargement: Chargement::EnCours(reception),
            terme: String::new(),
            focus: false,
            creer: None,
            lances: Vec::new(),
            retour: false,
            avec_artworks: true,
        };
        assert!(ecran.lances.is_empty());

        // Ce que fait le clic, sans passer par le rendu.
        for code in ["RA05", "LOCR-JP", "RA05"] {
            if !ecran.lances.contains(&code.to_owned()) {
                ecran.lances.push(code.to_owned());
            }
            ecran.creer = Some(code.to_owned());
            assert_eq!(ecran.creation_demandee().as_deref(), Some(code));
        }
        assert_eq!(
            ecran.lances,
            vec!["RA05".to_owned(), "LOCR-JP".to_owned()],
            "deux clics sur le même set n'en font qu'une entrée"
        );
    }

    #[test]
    fn les_demandes_se_consomment() {
        let (_envoi, reception) = std::sync::mpsc::channel::<Vec<SetDisponible>>();
        let mut ecran = EcranSelecteur {
            chargement: Chargement::EnCours(reception),
            terme: String::new(),
            focus: false,
            creer: Some("RA02".to_owned()),
            lances: vec!["RA02".to_owned()],
            retour: true,
            avec_artworks: true,
        };
        assert_eq!(ecran.creation_demandee().as_deref(), Some("RA02"));
        assert_eq!(ecran.creation_demandee(), None, "une seule fois");
        assert!(ecran.retour_demande());
        assert!(!ecran.retour_demande(), "une seule fois");
    }
}
