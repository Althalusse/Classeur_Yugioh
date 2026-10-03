// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Le premier lancement — quand `cardinfo.db` n'existe pas encore.
//!
//! Portage de la PHASE 1 de `main.py` et de `ui/init_window.py` (208 l.).
//!
//! # Ce qui manquait
//!
//! Le mécanisme d'initialisation était porté depuis le 2026-08-26, et son
//! bouton depuis le 2026-09-04. Restait ceci : **rien ne se passait au premier
//! démarrage**. L'application ouvrait un accueil vide, sans classeur, sans
//! dire pourquoi — et la seule façon d'en sortir était de trouver Options ▸
//! Base de référence. Le Python, lui, ouvre une fenêtre dédiée avant même de
//! construire son interface principale.
//!
//! # Une différence de déclencheur, et elle compte
//!
//! Le Python teste la présence d'un **drapeau**, `first_run.flag`, écrit après
//! la première initialisation réussie. Le défaut se voit tout de suite :
//! supprimer `cardinfo.db` ne fait pas revenir l'écran, puisque le drapeau,
//! lui, est toujours là. L'utilisateur se retrouve devant une application
//! muette avec un fichier en moins.
//!
//! Ici, le déclencheur est **l'état réel** — `cardinfo.db` absente ou vide,
//! via [`ygo_app::maj::etat_local`]. Un fichier supprimé ramène l'écran ; une
//! base construite le fait disparaître. Rien à tenir à jour, donc rien qui
//! puisse mentir.
//!
//! # La construction ne se demande pas
//!
//! Première version : un bouton « Construire la base », et l'écran attendait.
//! Retour de l'utilisateur, le 2026-09-05 : « aucune demande à faire, elle
//! doit se créer dans tous les cas pour fonctionner, c'est mandatory ».
//!
//! Il a raison, et la question était mal posée. Sans `cardinfo.db`
//! l'application ne sait rien faire de neuf : ni créer un classeur, ni
//! compléter des artworks, ni trier par rareté. Demander l'autorisation d'une
//! chose sans laquelle rien ne marche, c'est faire porter à l'utilisateur une
//! décision qui n'en est pas une — et lui coûter un clic pour la seule issue
//! possible.
//!
//! La construction part donc **à l'ouverture de l'écran**. Il ne reste qu'à
//! montrer où l'on en est.
//!
//! # La porte de sortie, et quand elle apparaît
//!
//! Le Python continue « en mode dégradé » si l'initialisation échoue. On garde
//! cette porte, mais on ne l'ouvre qu'**après un échec** — hors ligne,
//! typiquement. Avant, elle n'aurait aucun sens : proposer d'abandonner un
//! téléchargement qui vient de commencer et qui dure quarante secondes, c'est
//! inviter à se priver de l'application. Après, elle en a un : sans réseau, il
//! vaut mieux entrer et consulter ses classeurs existants que rester devant un
//! écran qu'on ne peut pas satisfaire.

use eframe::egui;
use ygo_core::paths::Paths;

use crate::telechargements::{Etat, PhaseBase};

/// L'écran de premier lancement.
pub struct EcranInitialisation {
    paths: Paths,
    /// La phase en cours, poussée par l'application.
    phase: Option<PhaseBase>,
    /// Part de la phase en cours, quand elle est connue.
    part: Option<f32>,
    /// Le travail est en route — l'écran montre l'avancement, pas de bouton.
    lancee: bool,
    /// Le fil a confirmé au moins une phase.
    ///
    /// Sans ce témoin, la toute première frame — où l'état est encore vide,
    /// le fil n'ayant pas eu le temps de répondre — passerait pour un échec,
    /// et la porte de sortie s'ouvrirait avant le premier octet reçu.
    confirmee: bool,
    /// Ce qui a échoué, s'il y a eu un échec.
    erreur: Option<String>,
    /// L'utilisateur a demandé la construction.
    demande: bool,
    /// L'utilisateur veut passer outre.
    ignore: bool,
}

impl std::fmt::Debug for EcranInitialisation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EcranInitialisation")
            .field("phase", &self.phase)
            .field("lancee", &self.lancee)
            .field("confirmee", &self.confirmee)
            .finish_non_exhaustive()
    }
}

impl EcranInitialisation {
    /// Ouvre l'écran.
    #[must_use]
    pub fn ouvrir(paths: Paths) -> Self {
        Self {
            paths,
            phase: None,
            part: None,
            // `lancee` dès l'ouverture, comme `demande` : la construction
            // part toute seule, et l'écran n'a jamais d'état « en attente
            // d'un clic ».
            lancee: true,
            confirmee: false,
            erreur: None,
            demande: true,
            ignore: false,
        }
    }

    /// Faut-il montrer cet écran pour cette installation ?
    ///
    /// Sur l'**état réel** de la base, pas sur un drapeau — cf. le module.
    #[must_use]
    pub fn necessaire(paths: &Paths) -> bool {
        !ygo_app::maj::etat_local(paths).base_presente
    }

    /// Reprend la demande de construction.
    ///
    /// Vraie **dès la première frame** : la construction n'attend pas de clic
    /// (cf. le module). L'appelant la reprend une fois et la lance.
    pub fn construction_demandee(&mut self) -> bool {
        std::mem::take(&mut self.demande)
    }

    /// Reprend la demande de passer outre.
    pub fn passer_outre(&mut self) -> bool {
        std::mem::take(&mut self.ignore)
    }

    /// Pousse l'avancement du fil de travail.
    ///
    /// L'écran ne lit ni le réseau ni le disque : il reçoit l'état de la même
    /// machine que la bande de progression, et n'en tire que ce qu'il montre.
    pub fn signaler(&mut self, etat: &Etat) {
        if let Some(crate::telechargements::Etape::Base(phase)) =
            etat.etape(crate::telechargements::CODE_BASE)
        {
            self.phase = Some(phase);
            self.lancee = true;
            self.confirmee = true;
            let cumul = etat.cumul();
            self.part = (cumul.total > 0).then(|| {
                #[allow(clippy::cast_precision_loss)]
                {
                    cumul.faites as f32 / cumul.total as f32
                }
            });
        } else if self.lancee && self.confirmee {
            // La tâche s'est refermée après avoir vraiment commencé.
            // L'application bascule sur l'accueil dès le succès : si l'on est
            // encore ici, c'est que ça n'a pas marché.
            self.phase = None;
            self.part = None;
            self.lancee = false;
            self.confirmee = false;
            self.erreur = etat.dernier().map(ToOwned::to_owned);
        }
    }
}

impl eframe::App for EcranInitialisation {
    fn ui(&mut self, racine: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(racine, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(48.0);
                ui.vertical_centered(|ui| {
                    ui.heading("Bienvenue");
                    ui.add_space(10.0);
                    ui.label(
                        "L'application a besoin de sa base de référence : les cartes, \
                         les sets, les illustrations et les raretés.",
                    );
                    ui.small(
                        "Elle se construit une fois, puis se met à jour quand la source bouge.",
                    );
                });
                ui.add_space(24.0);

                let largeur = 560.0_f32.min(ui.available_width() - 24.0);
                ui.vertical_centered(|ui| {
                    ui.allocate_ui(egui::vec2(largeur, 0.0), |ui| {
                        self.etapes(ui);
                        ui.add_space(16.0);
                        self.actions(ui);
                    });
                });
                ui.add_space(24.0);
                ui.vertical_centered(|ui| {
                    ui.small(self.paths.cardinfo_db().display().to_string());
                });
            });
        });
    }
}

impl EcranInitialisation {
    /// Les cinq phases, et où l'on en est.
    ///
    /// Le Python déroulait un journal texte. Une liste des étapes **connue
    /// d'avance** dit une chose de plus, et c'est celle qui fait patienter :
    /// combien il en reste.
    fn etapes(&self, ui: &mut egui::Ui) {
        egui::Frame::group(ui.style())
            .corner_radius(8.0)
            .inner_margin(12.0)
            .show(ui, |ui| {
                ui.set_width(ui.available_width() - 24.0);
                for phase in PhaseBase::toutes() {
                    let etat = self.etat_de(phase);
                    ui.horizontal(|ui| {
                        ui.label(etat.marque());
                        ui.add_space(4.0);
                        let titre = egui::RichText::new(phase.libelle());
                        ui.label(if etat == EtatEtape::EnCours {
                            titre.strong()
                        } else {
                            titre.weak()
                        });
                        ui.add_space(6.0);
                        ui.small(phase.detail());
                    });
                    if etat == EtatEtape::EnCours {
                        ui.add_space(2.0);
                        let barre = match self.part {
                            Some(part) => {
                                egui::ProgressBar::new(part).text(format!("{:.0} %", part * 100.0))
                            }
                            // Une phase sans total connu montre une animation
                            // plutôt qu'une progression inventée.
                            None => egui::ProgressBar::new(0.0).animate(true),
                        };
                        ui.add_sized([ui.available_width(), 16.0], barre);
                    }
                    ui.add_space(4.0);
                }
            });
    }

    /// Les boutons, et le message d'échec s'il y en a eu un.
    fn actions(&mut self, ui: &mut egui::Ui) {
        // Tant que ça travaille, il n'y a rien à décider — donc aucun bouton.
        if self.lancee {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.add_space(6.0);
                ui.small("environ 35 Mo — moins d'une minute");
            });
            return;
        }

        // On n'arrive ici qu'après un échec : c'est le seul cas où l'écran a
        // quelque chose à demander.
        if let Some(erreur) = self.erreur.clone() {
            ui.colored_label(ui.visuals().error_fg_color, erreur);
            ui.small("Vérifiez votre connexion, puis réessayez.");
            ui.add_space(8.0);
        }
        ui.horizontal(|ui| {
            if ui.button("⟳ Réessayer").clicked() {
                self.erreur = None;
                self.demande = true;
                self.lancee = true;
            }
            ui.add_space(8.0);
            if ui
                .button("Continuer sans base")
                .on_hover_text(
                    "Les classeurs déjà créés restent lisibles, mais il sera \
                     impossible d'en créer de nouveaux tant que la base manque.",
                )
                .clicked()
            {
                self.ignore = true;
            }
        });
    }

    /// Où en est une phase donnée.
    fn etat_de(&self, phase: PhaseBase) -> EtatEtape {
        match self.phase {
            None => EtatEtape::Attente,
            Some(courante) => match phase.rang().cmp(&courante.rang()) {
                std::cmp::Ordering::Less => EtatEtape::Faite,
                std::cmp::Ordering::Equal => EtatEtape::EnCours,
                std::cmp::Ordering::Greater => EtatEtape::Attente,
            },
        }
    }
}

/// Où en est une étape de la liste.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EtatEtape {
    /// Pas encore commencée.
    Attente,
    /// Celle du moment.
    EnCours,
    /// Franchie.
    Faite,
}

impl EtatEtape {
    /// La marque en tête de ligne.
    fn marque(self) -> &'static str {
        match self {
            Self::Attente => "○",
            Self::EnCours => "▶",
            Self::Faite => "✔",
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;
    use crate::telechargements::Evenement;

    fn ecran() -> (tempfile::TempDir, EcranInitialisation) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(tmp.path());
        (tmp, EcranInitialisation::ouvrir(paths))
    }

    /// Le déclencheur est l'état réel de la base, pas un drapeau — c'est la
    /// divergence assumée avec le `first_run.flag` du Python.
    #[test]
    fn l_ecran_suit_la_presence_de_la_base_et_pas_un_drapeau() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(tmp.path());
        assert!(EcranInitialisation::necessaire(&paths));

        paths.creer_dossiers().unwrap();
        std::fs::write(paths.cardinfo_db(), b"une base").unwrap();
        assert!(!EcranInitialisation::necessaire(&paths));

        // Et une base supprimée le ramène : chez le Python, le drapeau
        // survivait à la suppression et l'écran ne revenait jamais.
        std::fs::remove_file(paths.cardinfo_db()).unwrap();
        assert!(EcranInitialisation::necessaire(&paths));
    }

    /// Une base de zéro octet n'est pas une base : c'est ce que laisse une
    /// initialisation interrompue.
    #[test]
    fn une_base_vide_ramene_l_ecran() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(tmp.path());
        paths.creer_dossiers().unwrap();
        std::fs::write(paths.cardinfo_db(), b"").unwrap();
        assert!(EcranInitialisation::necessaire(&paths));
    }

    /// Le retour de l'utilisateur du 2026-09-05 : la base est obligatoire,
    /// donc sa construction ne se demande pas — elle part à l'ouverture.
    #[test]
    fn la_construction_part_sans_qu_on_la_demande() {
        let (_tmp, mut ecran) = ecran();
        assert!(
            ecran.construction_demandee(),
            "l'écran la demande dès sa première frame"
        );
        assert!(!ecran.construction_demandee(), "et une seule fois");
        assert!(ecran.lancee, "il se montre au travail, pas en attente");
    }

    #[test]
    fn la_demande_de_passer_outre_se_consomme() {
        let (_tmp, mut ecran) = ecran();
        assert!(!ecran.passer_outre());
        ecran.ignore = true;
        assert!(ecran.passer_outre());
        assert!(!ecran.passer_outre(), "et pas deux fois");
    }

    /// La liste marque comme faites toutes les phases avant la courante — ce
    /// qui est la seule information qui fasse patienter : combien il reste.
    #[test]
    fn les_phases_franchies_se_marquent_derriere_la_courante() {
        let (_tmp, mut ecran) = ecran();
        for phase in PhaseBase::toutes() {
            assert_eq!(ecran.etat_de(phase), EtatEtape::Attente);
        }

        let mut etat = Etat::default();
        etat.appliquer(&Evenement::BaseEnCours {
            phase: PhaseBase::Analyse,
        });
        ecran.signaler(&etat);

        assert_eq!(ecran.etat_de(PhaseBase::Telechargement), EtatEtape::Faite);
        assert_eq!(ecran.etat_de(PhaseBase::Catalogue), EtatEtape::Faite);
        assert_eq!(ecran.etat_de(PhaseBase::Analyse), EtatEtape::EnCours);
        assert_eq!(ecran.etat_de(PhaseBase::Artworks), EtatEtape::Attente);
        assert_eq!(ecran.etat_de(PhaseBase::Ecriture), EtatEtape::Attente);
    }

    /// Seule la première phase connaît son total ; les autres n'affichent pas
    /// de pourcentage inventé.
    #[test]
    fn la_part_n_est_connue_que_lorsqu_un_total_l_est() {
        let (_tmp, mut ecran) = ecran();
        let mut etat = Etat::default();
        etat.appliquer(&Evenement::BaseEnCours {
            phase: PhaseBase::Telechargement,
        });
        ecran.signaler(&etat);
        assert_eq!(ecran.part, None, "aucun total annoncé pour l'instant");

        etat.appliquer(&Evenement::Progression {
            code: crate::telechargements::CODE_BASE.to_owned(),
            faites: 25,
            total: 100,
        });
        ecran.signaler(&etat);
        assert_eq!(ecran.part, Some(0.25));

        etat.appliquer(&Evenement::BaseEnCours {
            phase: PhaseBase::Ecriture,
        });
        ecran.signaler(&etat);
        assert_eq!(ecran.part, None, "l'écriture ne sait pas où elle en est");
    }

    /// Un échec rend la main : le bouton redevient actionnable, et il dit
    /// « Réessayer ».
    #[test]
    fn un_echec_rouvre_le_bouton() {
        let (_tmp, mut ecran) = ecran();
        let mut etat = Etat::default();
        etat.appliquer(&Evenement::BaseEnCours {
            phase: PhaseBase::Telechargement,
        });
        ecran.signaler(&etat);
        assert!(
            ecran.lancee,
            "pendant le travail, aucun bouton n'est montré"
        );

        etat.appliquer(&Evenement::Echec {
            code: crate::telechargements::CODE_BASE.to_owned(),
            raison: "GitHub injoignable".to_owned(),
        });
        ecran.signaler(&etat);
        assert!(!ecran.lancee);
        assert!(
            ecran.erreur.unwrap_or_default().contains("injoignable"),
            "et il dit pourquoi"
        );
    }

    /// Le piège du départ automatique : entre la demande et le premier
    /// événement, l'état est encore vide. L'écran ne doit pas en conclure un
    /// échec — la porte de sortie s'ouvrirait avant le premier octet reçu.
    #[test]
    fn l_attente_du_premier_evenement_ne_vaut_pas_echec() {
        let (_tmp, mut ecran) = ecran();
        ecran.construction_demandee();
        for _ in 0..3 {
            ecran.signaler(&Etat::default());
        }
        assert!(ecran.erreur.is_none(), "aucun échec inventé");
        assert!(ecran.lancee, "et l'écran se tient toujours au travail");
    }
}
