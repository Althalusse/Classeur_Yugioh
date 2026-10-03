// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! La corbeille : voir, restaurer, effacer pour de bon.
//!
//! Sans écran, la corbeille n'était qu'un dossier — la promesse « rien n'est
//! perdu » obligeait à ouvrir l'explorateur et à renommer un dossier à la
//! main. Une corbeille qu'on ne peut pas vider est aussi une corbeille qui
//! grossit sans qu'on le sache.
//!
//! # Vider en entier, comme celle de Windows
//!
//! Effacer vingt entrées une par une, avec une confirmation chacune, c'est
//! vingt occasions de cliquer trop vite sur la vingt-et-unième. Le bouton
//! « Tout vider » fait le tour en une fois — et sa confirmation **totalise ce
//! qu'elle emporte** : le nombre d'entrées, les codes, et surtout les
//! exemplaires possédés, qui sont la seule chose qui n'existe nulle part
//! ailleurs.
//!
//! Le vidage ne s'arrête pas au premier obstacle. Un dossier verrouillé par
//! l'explorateur fait échouer le sien et rien d'autre ; le bilan **nomme**
//! ceux qui restent, plutôt que de laisser une corbeille à moitié vide sans
//! dire laquelle des vingt a bloqué.
//!
//! # Le bouton n'apparaît que si elle contient quelque chose
//!
//! Un bouton « Corbeille (0) » en permanence sur l'accueil occuperait une
//! place fixe pour une action rare. Il n'est montré que lorsqu'il y a quelque
//! chose dedans, et il **dit combien** — c'est le seul moment où l'information
//! est utile.

use std::path::PathBuf;

use eframe::egui;
use ygo_app::suppression::{self, Rebut};
use ygo_core::paths::Paths;

/// L'écran de la corbeille.
pub struct EcranCorbeille {
    paths: Paths,
    rebuts: Vec<Rebut>,
    /// L'entrée dont l'effacement définitif est en cours de confirmation.
    a_effacer: Option<Rebut>,
    /// Un vidage complet est en cours de confirmation, avec ce qu'il coûte.
    a_tout_vider: Option<suppression::Ardoise>,
    retour: bool,
    erreur: Option<String>,
    /// Un classeur a été restauré : l'accueil doit se relire.
    restauration: bool,
}

impl std::fmt::Debug for EcranCorbeille {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EcranCorbeille")
            .field("rebuts", &self.rebuts.len())
            .finish_non_exhaustive()
    }
}

impl EcranCorbeille {
    /// Lit la corbeille.
    #[must_use]
    pub fn ouvrir(paths: Paths) -> Self {
        let rebuts = suppression::rebuts(&paths);
        Self {
            paths,
            rebuts,
            a_effacer: None,
            a_tout_vider: None,
            retour: false,
            erreur: None,
            restauration: false,
        }
    }

    /// Combien d'entrées dort dans la corbeille d'une installation.
    ///
    /// Sert au bouton de l'accueil, qui ne s'affiche que si le compte est
    /// non nul.
    #[must_use]
    pub fn compter(paths: &Paths) -> usize {
        suppression::contenu_corbeille(paths).len()
    }

    /// Reprend la demande de retour.
    pub fn retour_demande(&mut self) -> bool {
        std::mem::take(&mut self.retour)
    }

    /// Reprend le fait qu'un classeur a été restauré.
    pub fn restauration_faite(&mut self) -> bool {
        std::mem::take(&mut self.restauration)
    }

    fn relire(&mut self) {
        self.rebuts = suppression::rebuts(&self.paths);
    }
}

impl eframe::App for EcranCorbeille {
    fn ui(&mut self, racine: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.barre_du_haut(racine);
        egui::CentralPanel::default().show(racine, |ui| {
            if self.rebuts.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(96.0);
                    ui.heading("La corbeille est vide.");
                    ui.label("Les classeurs supprimés atterrissent ici avant d'être effacés.");
                });
                return;
            }

            let mut a_restaurer: Option<String> = None;
            let mut a_confirmer: Option<Rebut> = None;

            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(8.0);
                for rebut in &self.rebuts {
                    egui::Frame::group(ui.style())
                        .corner_radius(6.0)
                        .inner_margin(10.0)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width() - 24.0);
                            ui.horizontal(|ui| {
                                ui.add_sized(
                                    [110.0, 20.0],
                                    egui::Label::new(
                                        egui::RichText::new(rebut.code.as_str())
                                            .strong()
                                            .monospace(),
                                    ),
                                );
                                ui.add_space(8.0);
                                ui.label(
                                    egui::RichText::new(format!("supprimé le {}", rebut.date))
                                        .weak(),
                                );

                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui.button("Effacer définitivement").clicked() {
                                            a_confirmer = Some(rebut.clone());
                                        }
                                        ui.add_space(8.0);
                                        // Restaurer par-dessus un classeur
                                        // vivant écraserait ce qui a été saisi
                                        // depuis : le bouton est désactivé, et
                                        // il dit pourquoi.
                                        let bouton = ui.add_enabled(
                                            !rebut.code_repris,
                                            egui::Button::new("Restaurer"),
                                        );
                                        if bouton.clicked() {
                                            a_restaurer = Some(rebut.dossier.clone());
                                        }
                                        if rebut.code_repris {
                                            bouton.on_hover_text(format!(
                                                "Un classeur {} existe déjà",
                                                rebut.code
                                            ));
                                        }
                                        ui.add_space(12.0);
                                        ui.small(format!(
                                            "{} carte(s) · {} exemplaire(s)",
                                            rebut.lignes, rebut.exemplaires
                                        ));
                                    },
                                );
                            });
                        });
                    ui.add_space(4.0);
                }
            });

            if let Some(dossier) = a_restaurer {
                match suppression::restaurer(&self.paths, &dossier) {
                    Ok(_) => {
                        self.erreur = None;
                        self.restauration = true;
                        self.relire();
                    }
                    Err(e) => self.erreur = Some(e.to_string()),
                }
            }
            if let Some(rebut) = a_confirmer {
                self.a_effacer = Some(rebut);
            }
        });
        self.confirmation_effacement(racine.ctx());
        self.confirmation_vidage(racine.ctx());
    }
}

impl EcranCorbeille {
    fn barre_du_haut(&mut self, racine: &mut egui::Ui) {
        egui::Panel::top("entete-corbeille").show(racine, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("◀ Mes classeurs").clicked() {
                    self.retour = true;
                }
                ui.add_space(8.0);
                ui.heading("Corbeille");
                ui.add_space(12.0);
                ui.small(suppression::corbeille(&self.paths).display().to_string());
                // À droite, la seule action qui porte sur l'ensemble.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(!self.rebuts.is_empty(), egui::Button::new("🗑 Tout vider"))
                        .clicked()
                    {
                        self.a_tout_vider = Some(suppression::ardoise(&self.paths));
                    }
                });
            });
            if let Some(erreur) = self.erreur.clone() {
                ui.colored_label(ui.visuals().error_fg_color, erreur);
            }
            ui.add_space(8.0);
        });
    }

    /// L'effacement définitif, lui, n'a pas de filet : il faut le dire.
    fn confirmation_effacement(&mut self, ctx: &egui::Context) {
        let Some(rebut) = self.a_effacer.clone() else {
            return;
        };
        let mut ouverte = true;
        let mut confirme = false;
        egui::Window::new(format!("Effacer {} définitivement ?", rebut.code))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .open(&mut ouverte)
            .show(ctx, |ui| {
                ui.set_min_width(420.0);
                ui.add_space(4.0);
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    "Cette fois, rien ne se rattrape.",
                );
                ui.add_space(6.0);
                ui.label(format!(
                    "{} carte(s), {} exemplaire(s) possédé(s), supprimé le {}.",
                    rebut.lignes, rebut.exemplaires, rebut.date
                ));
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button("Annuler").clicked() {
                        self.a_effacer = None;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Effacer définitivement").clicked() {
                            confirme = true;
                        }
                    });
                });
                ui.add_space(4.0);
            });
        if confirme {
            match suppression::vider(&self.paths, &rebut.dossier) {
                Ok(()) => self.erreur = None,
                Err(e) => self.erreur = Some(e.to_string()),
            }
            self.a_effacer = None;
            self.relire();
        } else if !ouverte {
            self.a_effacer = None;
        }
    }
}

impl EcranCorbeille {
    /// Le vidage complet : une seule fenêtre pour tout ce qui va partir.
    ///
    /// Elle totalise plutôt qu'elle n'énumère — vingt lignes de dossiers ne
    /// se lisent pas avant de cliquer. Ce qui compte, ce sont les
    /// exemplaires : les lignes de classeur se recréent, eux non.
    fn confirmation_vidage(&mut self, ctx: &egui::Context) {
        let Some(ardoise) = self.a_tout_vider.clone() else {
            return;
        };
        let mut ouverte = true;
        let mut confirme = false;
        egui::Window::new("Vider toute la corbeille ?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .open(&mut ouverte)
            .show(ctx, |ui| {
                ui.set_min_width(460.0);
                ui.add_space(4.0);
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    "Cette fois, rien ne se rattrape.",
                );
                ui.add_space(6.0);
                ui.label(format!(
                    "{} entrée(s), {} ligne(s) de classeur.",
                    ardoise.entrees(),
                    ardoise.lignes
                ));
                if ardoise.irreversible() {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        format!(
                            "{} exemplaire(s) possédé(s) — ils n'existent nulle part ailleurs.",
                            ardoise.exemplaires
                        ),
                    );
                }
                ui.add_space(4.0);
                ui.small(ardoise.codes().join(", "));
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button("Annuler").clicked() {
                        self.a_tout_vider = None;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Tout vider").clicked() {
                            confirme = true;
                        }
                    });
                });
                ui.add_space(4.0);
            });
        if confirme {
            let bilan = suppression::vider_tout(&self.paths);
            // Un échec partiel se dit, et se dit en nommant : « 3 entrées
            // n'ont pas pu être effacées » sans lesquelles n'aiderait
            // personne à comprendre quoi débloquer.
            self.erreur = (!bilan.complet()).then(|| {
                let noms: Vec<&str> = bilan.echecs.iter().map(|(d, _)| d.as_str()).collect();
                format!(
                    "{} entrée(s) effacée(s). Restent, verrouillées : {}",
                    bilan.effacees.len(),
                    noms.join(", ")
                )
            });
            self.a_tout_vider = None;
            self.relire();
        } else if !ouverte {
            self.a_tout_vider = None;
        }
    }
}

/// La racine de l'installation, pour l'appelant.
impl EcranCorbeille {
    /// Les chemins de l'installation.
    #[must_use]
    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    /// Le dossier de la corbeille, pour l'afficher.
    #[must_use]
    pub fn dossier(&self) -> PathBuf {
        suppression::corbeille(&self.paths)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    fn installation() -> (tempfile::TempDir, Paths) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(tmp.path());
        std::fs::create_dir_all(paths.classeurs()).unwrap();
        (tmp, paths)
    }

    fn classeur(paths: &Paths, code: &str) {
        std::fs::create_dir_all(paths.dossier_classeur(code)).unwrap();
        let conn = ygo_db::rusqlite::Connection::open(paths.classeur_db(code)).unwrap();
        conn.execute("CREATE TABLE cards (quantite INTEGER)", ())
            .unwrap();
        conn.execute("INSERT INTO cards VALUES (2)", ()).unwrap();
    }

    /// Le compte sert au bouton de l'accueil, qui ne s'affiche que s'il est
    /// non nul.
    #[test]
    fn le_compte_suit_ce_que_la_corbeille_contient() {
        let (_tmp, paths) = installation();
        assert_eq!(EcranCorbeille::compter(&paths), 0);

        classeur(&paths, "RA05");
        ygo_app::suppression::vers_corbeille(&paths, "RA05", "h").unwrap();
        assert_eq!(EcranCorbeille::compter(&paths), 1);
    }

    #[test]
    fn l_ecran_lit_la_corbeille_a_l_ouverture() {
        let (_tmp, paths) = installation();
        classeur(&paths, "RA05");
        ygo_app::suppression::vers_corbeille(&paths, "RA05", "2026-08-28_120000").unwrap();

        let ecran = EcranCorbeille::ouvrir(paths);
        assert_eq!(ecran.rebuts.len(), 1);
        assert_eq!(ecran.rebuts[0].code, "RA05");
        assert_eq!(ecran.rebuts[0].exemplaires, 2);
        assert!(!ecran.rebuts[0].code_repris);
    }

    #[test]
    fn les_demandes_se_consomment() {
        let (_tmp, paths) = installation();
        let mut ecran = EcranCorbeille::ouvrir(paths);
        ecran.retour = true;
        ecran.restauration = true;
        assert!(ecran.retour_demande());
        assert!(!ecran.retour_demande(), "une seule fois");
        assert!(ecran.restauration_faite());
        assert!(!ecran.restauration_faite(), "une seule fois");
    }
}
