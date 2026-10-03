// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Le panneau d'import/export Scanflip, partagé par les écrans.
//!
//! # Pourquoi un module plutôt qu'un écran
//!
//! Il vivait d'abord **dans** l'écran de l'inventaire, derrière un bouton
//! nommé « Toutes mes cartes ». Rien sur l'accueil ne laissait deviner que
//! l'application savait importer, et le nom du bouton n'évoquait pas un
//! import : la fonction existait sans être trouvable.
//!
//! Le remède n'est pas de recopier deux cents lignes d'affichage dans un
//! second écran — deux copies divergent au premier correctif. C'est un
//! panneau autonome, que l'accueil et l'inventaire montrent l'un comme
//! l'autre, avec le même comportement et un seul endroit à corriger.
//!
//! # Le sélecteur de fichier est celui du système
//!
//! Le chemin s'écrivait à la main, et rien d'autre ne le permettait :
//! l'utilisateur devait aller chercher son fichier dans l'explorateur,
//! copier le chemin, puis le recoller ici. Un bouton **Parcourir…** ouvre
//! désormais la boîte de dialogue **native** — celle de Windows, avec ses
//! emplacements récents et ses raccourcis, pas un sélecteur redessiné en
//! egui qui n'en connaîtrait aucun.
//!
//! Elle s'ouvre **sur un fil à part**, et le panneau consulte le résultat à
//! chaque image. Un appel bloquant depuis la boucle de rendu figerait la
//! fenêtre entière tant que l'utilisateur n'a pas choisi : pas de
//! redessin, pas de curseur d'attente, une application que Windows finit
//! par déclarer « ne répond pas » si la boîte reste ouverte assez
//! longtemps.
//!
//! Le champ de saisie **reste** : coller un chemin est parfois plus rapide
//! que naviguer, et un chemin reçu par message se colle sans détour.
//!
//! # Ce que le panneau ne fait pas
//!
//! Il n'écrit pas de classeur et ne recharge pas l'écran qui l'héberge : il
//! rend un [`Retour`] disant ce qui s'est passé, et l'écran en tire les
//! conséquences. C'est la même séparation que partout ailleurs — le
//! composant dit, l'appelant décide.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};

use eframe::egui;

use ygo_core::paths::Paths;

/// Ce que le panneau montre, s'il montre quelque chose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Face {
    /// Rien : le panneau est fermé.
    #[default]
    Fermee,
    /// L'import et son rapport.
    Import,
    /// L'export et sa destination.
    Export,
}

/// Ce qu'une image du panneau a produit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Retour {
    /// Un message à afficher, et s'il s'agit d'une erreur.
    pub message: Option<(String, bool)>,
    /// Un import a été appliqué : les quantités ont changé.
    pub importe: bool,
    /// Des classeurs à créer, demandés par l'utilisateur.
    pub creations: Vec<String>,
}

impl Retour {
    /// Y a-t-il quelque chose à rapporter ?
    #[must_use]
    pub fn vide(&self) -> bool {
        self.message.is_none() && !self.importe && self.creations.is_empty()
    }
}

/// Le panneau d'import/export.
pub struct PanneauCsv {
    /// Un identifiant propre à l'écran hôte : deux panneaux ouverts dans
    /// deux écrans ne doivent pas partager l'état de disposition d'egui.
    id: &'static str,
    face: Face,
    chemin_import: String,
    chemin_export: String,
    /// Le rapport du dernier import analysé — c'est lui qu'on applique.
    rapport: Option<ygo_app::import::Rapport>,
    /// Une boîte de dialogue ouverte sur un fil, et ce qu'elle remplira.
    dialogue: Option<Dialogue>,
}

/// Une boîte de dialogue système en cours, et le champ qu'elle vise.
struct Dialogue {
    /// Le champ à remplir quand l'utilisateur aura choisi.
    cible: Face,
    /// Le fil rend `None` si l'utilisateur a annulé.
    reception: Receiver<Option<PathBuf>>,
}

impl std::fmt::Debug for PanneauCsv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PanneauCsv")
            .field("id", &self.id)
            .field("face", &self.face)
            .finish_non_exhaustive()
    }
}

/// Ce fichier est-il un CSV ?
///
/// La casse de l'extension est indifférente : Windows écrit volontiers
/// `.CSV`, et refuser pour cela n'aiderait personne.
///
/// ```
/// use std::path::Path;
/// use ygo_ui::csv::est_csv;
/// assert!(est_csv(Path::new("collection.csv")));
/// assert!(est_csv(Path::new("collection.CSV")));
/// assert!(!est_csv(Path::new("image.png")));
/// assert!(!est_csv(Path::new("sans_extension")));
/// ```
#[must_use]
pub fn est_csv(chemin: &std::path::Path) -> bool {
    chemin
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("csv"))
}

/// Le nom de fichier proposé à l'export : la date du jour suffit à ne pas
/// écraser celui d'hier.
#[must_use]
pub fn destination_par_defaut(paths: &Paths) -> String {
    paths
        .export()
        .join(format!(
            "collection_{}.csv",
            chrono::Local::now().format("%Y-%m-%d")
        ))
        .display()
        .to_string()
}

/// Le dossier où ouvrir la boîte de dialogue, d'après ce qui est écrit.
///
/// Ce que l'utilisateur a déjà tapé vaut mieux que le dossier courant du
/// processus : rouvrir la boîte après une faute de frappe doit revenir là
/// où il en était, pas à la racine.
///
/// La fonction interroge le disque — un dossier qui n'existe pas n'est pas un
/// point de départ. L'exemple part donc du dossier temporaire du système,
/// qui existe partout, plutôt que d'un `/tmp` écrit en dur : ce dernier
/// n'existe pas sous Windows, et l'exemple y échouait.
///
/// ```
/// use ygo_ui::csv::dossier_de_depart;
/// let dossier = std::env::temp_dir();
/// let fichier = dossier.join("collection.csv");
/// // Un chemin complet : son dossier.
/// assert_eq!(
///     dossier_de_depart(&fichier.to_string_lossy()).as_deref(),
///     fichier.parent()
/// );
/// // Un dossier : lui-même.
/// assert_eq!(
///     dossier_de_depart(&dossier.to_string_lossy()),
///     Some(dossier)
/// );
/// // Rien d'exploitable : au système de décider.
/// assert_eq!(dossier_de_depart("   "), None);
/// assert_eq!(dossier_de_depart("collection.csv"), None);
/// ```
#[must_use]
pub fn dossier_de_depart(ecrit: &str) -> Option<PathBuf> {
    let chemin = Path::new(ecrit.trim());
    if ecrit.trim().is_empty() {
        return None;
    }
    if chemin.is_dir() {
        return Some(chemin.to_path_buf());
    }
    chemin
        .parent()
        .filter(|p| !p.as_os_str().is_empty() && p.is_dir())
        .map(Path::to_path_buf)
}

/// Le nom de fichier à proposer dans la boîte d'enregistrement.
///
/// Sans nom exploitable, on retombe sur `collection.csv` plutôt que de
/// laisser le champ vide : une boîte d'enregistrement sans nom oblige à
/// tout taper.
///
/// Le cas « c'est un dossier » se lit sur le disque, d'où le dossier
/// temporaire du système plutôt qu'un `/tmp` écrit en dur — voir la note de
/// [`dossier_de_depart`].
///
/// ```
/// use ygo_ui::csv::nom_de_depart;
/// let dossier = std::env::temp_dir();
/// let fichier = dossier.join("collection_2026-08-31.csv");
/// assert_eq!(
///     nom_de_depart(&fichier.to_string_lossy()),
///     "collection_2026-08-31.csv"
/// );
/// // Un dossier n'est pas un nom de fichier.
/// assert_eq!(nom_de_depart(&dossier.to_string_lossy()), "collection.csv");
/// assert_eq!(nom_de_depart(""), "collection.csv");
/// ```
#[must_use]
pub fn nom_de_depart(ecrit: &str) -> String {
    let chemin = Path::new(ecrit.trim());
    if chemin.is_dir() {
        return "collection.csv".to_owned();
    }
    chemin
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty())
        .map_or_else(|| "collection.csv".to_owned(), ToOwned::to_owned)
}

impl PanneauCsv {
    /// Un panneau fermé, prêt à s'ouvrir.
    #[must_use]
    pub fn nouveau(id: &'static str, paths: &Paths) -> Self {
        Self {
            id,
            face: Face::Fermee,
            chemin_import: String::new(),
            chemin_export: destination_par_defaut(paths),
            rapport: None,
            dialogue: None,
        }
    }

    /// La face montrée.
    #[must_use]
    pub fn face(&self) -> Face {
        self.face
    }

    /// Ouvre l'import, ou le referme s'il l'était déjà.
    ///
    /// Le même bouton ouvre et ferme : c'est ce qu'on attend d'un panneau
    /// latéral, et ça évite un second bouton « Fermer » que l'œil doit
    /// chercher ailleurs.
    pub fn basculer(&mut self, face: Face) {
        self.face = if self.face == face {
            Face::Fermee
        } else {
            face
        };
    }

    /// Referme le panneau.
    pub fn fermer(&mut self) {
        self.face = Face::Fermee;
    }

    /// Capte un CSV lâché sur la fenêtre.
    ///
    /// Ne fait que lire le contexte egui et déléguer à [`capter`](Self::capter),
    /// qui porte toute la règle et se teste sans fenêtre.
    pub fn capter_depot(&mut self, ctx: &egui::Context, paths: &Paths) -> bool {
        let deposes: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        self.capter(&deposes, paths)
    }

    /// Ce qu'un lot de fichiers déposés déclenche.
    ///
    /// Le panneau s'ouvre sur l'import avec le fichier déjà analysé, et la
    /// fonction rend `true` — l'écran hôte sait ainsi qu'il vient de se
    /// passer quelque chose, et peut effacer son message précédent.
    ///
    /// Un lot sans CSV ne réveille rien : on lâche des images sur une
    /// fenêtre de collection de cartes plus souvent qu'on ne le croit.
    pub fn capter(&mut self, deposes: &[PathBuf], paths: &Paths) -> bool {
        let Some(chemin) = deposes.iter().find(|p| est_csv(p)) else {
            return false;
        };
        self.chemin_import = chemin.display().to_string();
        self.face = Face::Import;
        self.analyser(paths);
        true
    }

    /// Lit et apparie le CSV désigné, sans rien écrire.
    fn analyser(&mut self, paths: &Paths) -> Option<(String, bool)> {
        let chemin = PathBuf::from(self.chemin_import.trim());
        match ygo_app::scanflip::lire(&chemin) {
            Ok(lecture) => {
                let bases = ygo_app::import::bases_pour(paths, &lecture.lignes);
                self.rapport = Some(ygo_app::import::planifier(&lecture.lignes, &bases));
                None
            }
            Err(e) => {
                self.rapport = None;
                Some((e.to_string(), true))
            }
        }
    }

    /// Ouvre la boîte de dialogue du système, sur un fil.
    ///
    /// Rien ne se passe si une boîte est déjà ouverte : deux dialogues
    /// modaux concurrents, c'est un clic perdu et un fil de plus.
    fn ouvrir_dialogue(&mut self, cible: Face) {
        if self.dialogue.is_some() || cible == Face::Fermee {
            return;
        }
        let (envoi, reception) = std::sync::mpsc::channel();
        let ecrit = match cible {
            Face::Export => self.chemin_export.clone(),
            _ => self.chemin_import.clone(),
        };
        std::thread::spawn(move || {
            let mut boite = rfd::FileDialog::new().add_filter("CSV Scanflip", &["csv"]);
            if let Some(dossier) = dossier_de_depart(&ecrit) {
                boite = boite.set_directory(dossier);
            }
            let choisi = if cible == Face::Export {
                boite.set_file_name(nom_de_depart(&ecrit)).save_file()
            } else {
                boite.pick_file()
            };
            // L'échec d'envoi veut dire que le panneau a été détruit
            // entre-temps : il n'y a plus personne à prévenir, et ce n'est
            // pas une erreur.
            drop(envoi.send(choisi));
        });
        self.dialogue = Some(Dialogue { cible, reception });
    }

    /// Relève ce qu'une boîte de dialogue a rendu, s'il y a quelque chose.
    ///
    /// Rend le chemin choisi. Une annulation ferme le dialogue sans rien
    /// changer aux champs — l'utilisateur retrouve ce qu'il avait écrit.
    fn relever_dialogue(&mut self) -> Option<(Face, PathBuf)> {
        let dialogue = self.dialogue.as_ref()?;
        let cible = dialogue.cible;
        match dialogue.reception.try_recv() {
            Ok(choisi) => {
                self.dialogue = None;
                choisi.map(|chemin| (cible, chemin))
            }
            // Le fil vit encore : la boîte est toujours à l'écran.
            Err(TryRecvError::Empty) => None,
            // Le fil est mort sans répondre — on ne restera pas à
            // l'attendre indéfiniment.
            Err(TryRecvError::Disconnected) => {
                self.dialogue = None;
                None
            }
        }
    }

    /// Le bouton « Parcourir… », et ce que son résultat déclenche.
    ///
    /// Le sondage se fait ici plutôt qu'en tête d'image : tant qu'une boîte
    /// est ouverte, il faut redemander un redessin, sans quoi egui
    /// s'endort et le chemin choisi n'apparaît qu'au prochain mouvement de
    /// souris.
    fn bouton_parcourir(&mut self, ui: &mut egui::Ui, cible: Face) -> Option<PathBuf> {
        if ui.button("Parcourir…").clicked() {
            self.ouvrir_dialogue(cible);
        }
        if self.dialogue.is_some() {
            ui.ctx().request_repaint();
        }
        match self.relever_dialogue() {
            Some((face, chemin)) if face == cible => Some(chemin),
            _ => None,
        }
    }

    /// Affiche le panneau. Ne fait rien s'il est fermé.
    ///
    /// `francais` décide de la langue de l'export — elle suit la case FR de
    /// l'écran hôte, pour que ce que l'utilisateur voit à l'écran soit ce
    /// qu'il obtient dans le fichier.
    pub fn afficher(&mut self, racine: &mut egui::Ui, paths: &Paths, francais: bool) -> Retour {
        match self.face {
            Face::Fermee => Retour::default(),
            Face::Import => self.face_import(racine, paths),
            Face::Export => self.face_export(racine, paths, francais),
        }
    }

    fn face_import(&mut self, racine: &mut egui::Ui, paths: &Paths) -> Retour {
        let mut retour = Retour::default();
        let id = self.id;
        egui::Panel::right(format!("import-{id}"))
            .default_size(430.0)
            .show(racine, |ui| {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.heading("Importer");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Fermer").clicked() {
                            self.face = Face::Fermee;
                        }
                    });
                });
                ui.add_space(4.0);
                ui.label(
                    "Choisissez un CSV Scanflip, déposez-le sur la fenêtre, \
                          ou écrivez son chemin :",
                );
                // Le bouton d'abord : c'est la voie courte, et l'œil la
                // trouve avant d'avoir jugé le champ de saisie.
                let mut choisi = None;
                let mut valide = false;
                ui.horizontal(|ui| {
                    choisi = self.bouton_parcourir(ui, Face::Import);
                    let saisie = ui.add(
                        egui::TextEdit::singleline(&mut self.chemin_import)
                            .hint_text("C:\\…\\collection.csv")
                            .desired_width(f32::INFINITY),
                    );
                    // Entrée dans le champ vaut « Analyser » — mais Entrée
                    // ailleurs dans la fenêtre ne doit rien déclencher.
                    valide = saisie.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                });
                // Un fichier choisi s'analyse tout de suite : l'utilisateur
                // vient de dire lequel, redemander « Analyser » serait un
                // clic pour rien.
                if let Some(chemin) = choisi {
                    self.chemin_import = chemin.display().to_string();
                    retour.message = self.analyser(paths);
                }
                ui.add_space(6.0);
                if ui.button("Analyser").clicked() || valide {
                    retour.message = self.analyser(paths);
                }
                ui.separator();

                let Some(rapport) = self.rapport.clone() else {
                    ui.weak("Rien d'analysé pour l'instant.");
                    ui.add_space(8.0);
                    return;
                };

                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.label(format!(
                        "{} ligne(s) lue(s) · {} appariée(s) · {} refusée(s)",
                        rapport.lues,
                        rapport.appariees(),
                        rapport.refusees.len()
                    ));
                    ui.label(format!(
                        "{} ligne(s) de classeur seraient écrites.",
                        rapport.ecritures.len()
                    ));

                    // Les classeurs absents d'abord : les créer change tout
                    // le reste du rapport, autant le proposer avant que
                    // l'utilisateur ne lise les refus qui en découlent.
                    let absents = rapport.classeurs_absents();
                    if !absents.is_empty() {
                        ui.add_space(8.0);
                        ui.colored_label(
                            ui.visuals().warn_fg_color,
                            format!(
                                "{} classeur(s) du CSV absent(s) : {}",
                                absents.len(),
                                absents.join(", ")
                            ),
                        );
                        ui.small(
                            "Toutes leurs lignes sont refusées. Les créer d'abord \
                             change ce que l'import trouvera.",
                        );
                        if ui
                            .button(format!("Créer ces {} classeur(s)", absents.len()))
                            .clicked()
                        {
                            retour.creations.clone_from(&absents);
                            retour.message = Some((
                                format!(
                                    "Création lancée : {} — relancez l'analyse quand \
                                     la barre de progression a fini",
                                    absents.join(", ")
                                ),
                                false,
                            ));
                        }
                    }

                    // Ce que le repli des sous-jeux a rattaché. Deviner
                    // en silence serait pire que refuser : l'utilisateur
                    // doit pouvoir vérifier chaque rattachement.
                    if !rapport.resolues.is_empty() {
                        ui.add_space(8.0);
                        egui::CollapsingHeader::new(format!(
                            "{} code(s) sans lettre de sous-jeu, retrouvé(s) par le nom",
                            rapport.resolues.len()
                        ))
                        .show(ui, |ui| {
                            ui.small(
                                "Votre fichier écrit « LDK2-FR01 » là où le set numérote \
                                 chaque sous-deck à part (J, K, S, Y). Le nom de la carte \
                                 a désigné un tirage et un seul.",
                            );
                            for r in &rapport.resolues {
                                ui.small(format!(
                                    "ligne {} · {} → {} · {}",
                                    r.numero, r.code_csv, r.code_retenu, r.nom
                                ));
                            }
                        });
                    }

                    if !rapport.refusees.is_empty() {
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new("Refusées").strong());
                        let mut par_cause: std::collections::BTreeMap<&str, Vec<&_>> =
                            std::collections::BTreeMap::new();
                        for r in &rapport.refusees {
                            par_cause.entry(r.refus.libelle()).or_default().push(r);
                        }
                        for (cause, lignes) in par_cause {
                            egui::CollapsingHeader::new(format!("{} — {cause}", lignes.len()))
                                .id_salt(cause)
                                .show(ui, |ui| {
                                    for r in lignes {
                                        ui.small(format!(
                                            "ligne {} · {} {} · {}",
                                            r.numero, r.code, r.rarete, r.nom
                                        ));
                                    }
                                });
                        }
                    }

                    let avec_perte = rapport.fusions_avec_perte().count();
                    if avec_perte > 0 {
                        ui.add_space(8.0);
                        egui::CollapsingHeader::new(format!(
                            "{avec_perte} fusion(s) effacent une information"
                        ))
                        .show(ui, |ui| {
                            ui.small(
                                "La base ne garde pas la langue d'un exemplaire : deux \
                                 langues d'un même tirage se fondent. L'état et \
                                 l'édition, eux, sont gardés exemplaire par exemplaire.",
                            );
                            for f in rapport.fusions_avec_perte() {
                                let mut quoi = Vec::new();
                                if f.perte.langue {
                                    quoi.push("langue");
                                }
                                ui.small(format!(
                                    "{} {} · lignes {:?} · {}",
                                    f.classeur,
                                    f.code,
                                    f.numeros,
                                    quoi.join(", ")
                                ));
                            }
                        });
                    }

                    ui.add_space(12.0);
                    if ui
                        .add_enabled(
                            !rapport.ecritures.is_empty(),
                            egui::Button::new("Appliquer l'import"),
                        )
                        .clicked()
                    {
                        match ygo_app::import::appliquer(paths, &rapport) {
                            Ok(bilan) => {
                                retour.message = Some((
                                    format!(
                                        "Import appliqué — {} ligne(s) dans {}",
                                        bilan.ecrites,
                                        bilan.classeurs.join(", ")
                                    ),
                                    false,
                                ));
                                retour.importe = true;
                                self.rapport = None;
                                self.face = Face::Fermee;
                            }
                            Err(e) => retour.message = Some((e.to_string(), true)),
                        }
                    }
                    ui.small("Les lignes que le CSV ne nomme pas gardent leur quantité.");
                    ui.add_space(8.0);
                });
            });
        retour
    }

    fn face_export(&mut self, racine: &mut egui::Ui, paths: &Paths, francais: bool) -> Retour {
        let mut retour = Retour::default();
        let id = self.id;
        egui::Panel::right(format!("export-{id}"))
            .default_size(430.0)
            .show(racine, |ui| {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.heading("Exporter");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Fermer").clicked() {
                            self.face = Face::Fermee;
                        }
                    });
                });
                ui.add_space(4.0);
                ui.label("Fichier de destination :");
                let mut choisi = None;
                ui.horizontal(|ui| {
                    choisi = self.bouton_parcourir(ui, Face::Export);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.chemin_export)
                            .desired_width(f32::INFINITY),
                    );
                });
                // Choisir une destination ne l'écrit pas : la boîte
                // d'enregistrement a déjà demandé confirmation d'un
                // écrasement, mais c'est « Écrire le fichier » qui engage.
                if let Some(chemin) = choisi {
                    self.chemin_export = chemin.display().to_string();
                }
                ui.add_space(6.0);

                let (gardes, ecartes) = ygo_app::export::ecarter_ocg(&paths.classeurs_existants());
                if !ecartes.is_empty() {
                    ui.small(format!(
                        "{} classeur(s) OCG écarté(s) : {} — leur code est natif, \
                         mais la colonne Langue du format est unique pour tout le fichier.",
                        ecartes.len(),
                        ecartes.join(", ")
                    ));
                    ui.add_space(4.0);
                }

                let langue = if francais {
                    ygo_app::export::Langue::Francais
                } else {
                    ygo_app::export::Langue::Anglais
                };
                ui.small(format!(
                    "Langue du fichier : {} (elle suit la case FR de l'en-tête).",
                    langue.libelle()
                ));
                ui.add_space(8.0);

                if ui.button("Écrire le fichier").clicked() {
                    let chemin = PathBuf::from(self.chemin_export.trim());
                    // Les rangs se lisent sur la base entière des
                    // classeurs, pas sur les seules cartes possédées :
                    // c'est la même numérotation que l'import.
                    let resultat = ygo_app::export::bases_de(paths, &gardes)
                        .and_then(|bases| {
                            let rangs = ygo_app::export::rangs(&bases);
                            ygo_app::export::possedees_de(paths, &gardes)
                                .map(|p| ygo_app::export::lignes(&p, langue, &rangs))
                        })
                        .and_then(|lignes| {
                            ygo_app::scanflip::ecrire(&chemin, &lignes)
                                .map(|()| ygo_app::export::bilan(&lignes, ecartes.clone()))
                        });
                    match resultat {
                        Ok(bilan) => {
                            retour.message = Some((message_export(&bilan, &chemin), false));
                            self.face = Face::Fermee;
                        }
                        Err(e) => retour.message = Some((e.to_string(), true)),
                    }
                }
                ui.add_space(8.0);
            });
        retour
    }
}

/// Ce que l'export a produit, dit en une phrase.
///
/// Fonction pure, pour que les deux avertissements — raretés hors règles,
/// lignes sans état — soient vérifiables sans fenêtre. Ce sont eux qui
/// font la différence entre un fichier qui passera chez Scanflip et un
/// fichier dont des lignes seront rejetées sans explication.
#[must_use]
pub fn message_export(bilan: &ygo_app::export::Bilan, chemin: &std::path::Path) -> String {
    let mut texte = format!(
        "Export écrit — {} ligne(s), {} exemplaire(s) → {}",
        bilan.lignes,
        bilan.exemplaires,
        chemin.display()
    );
    if !bilan.raretes_refusees.is_empty() {
        texte.push_str(&format!(
            " · ATTENTION : {} hors des règles de Scanflip, ces lignes seront rejetées",
            bilan.raretes_refusees.join(", ")
        ));
    }
    if bilan.sans_etat > 0 {
        texte.push_str(&format!(
            " · {} ligne(s) sans état (filtre « sans état » de l'inventaire \
             pour le renseigner en masse)",
            bilan.sans_etat
        ));
    }
    texte
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    fn paths() -> (tempfile::TempDir, Paths) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(tmp.path());
        (tmp, paths)
    }

    /// Le même bouton ouvre et referme.
    #[test]
    fn un_bouton_bascule_sa_face() {
        let (_tmp, paths) = paths();
        let mut p = PanneauCsv::nouveau("essai", &paths);
        assert_eq!(p.face(), Face::Fermee);

        p.basculer(Face::Import);
        assert_eq!(p.face(), Face::Import);
        p.basculer(Face::Import);
        assert_eq!(p.face(), Face::Fermee, "le même bouton referme");

        // Passer d'une face à l'autre ne referme pas.
        p.basculer(Face::Import);
        p.basculer(Face::Export);
        assert_eq!(p.face(), Face::Export);
        p.fermer();
        assert_eq!(p.face(), Face::Fermee);
    }

    /// La boîte s'ouvre là où l'utilisateur en était, pas à la racine.
    #[test]
    fn la_boite_s_ouvre_sur_le_dossier_deja_ecrit() {
        let tmp = tempfile::tempdir().unwrap();
        let dossier = tmp.path().to_path_buf();
        let fichier = dossier.join("collection.csv");
        std::fs::write(&fichier, "").unwrap();

        // Un chemin de fichier : son dossier.
        assert_eq!(
            dossier_de_depart(&fichier.display().to_string()),
            Some(dossier.clone())
        );
        // Un dossier : lui-même.
        assert_eq!(
            dossier_de_depart(&dossier.display().to_string()),
            Some(dossier.clone())
        );
        // Les espaces autour ne comptent pas — un chemin collé en traîne.
        assert_eq!(
            dossier_de_depart(&format!("  {}  ", dossier.display())),
            Some(dossier)
        );
        // Rien d'exploitable : au système de choisir, pas à nous d'inventer.
        assert_eq!(dossier_de_depart(""), None);
        assert_eq!(dossier_de_depart("   "), None);
        assert_eq!(dossier_de_depart("/dossier/qui/n/existe/pas/x.csv"), None);
    }

    /// La boîte d'enregistrement propose toujours un nom.
    #[test]
    fn la_boite_d_enregistrement_propose_toujours_un_nom() {
        let tmp = tempfile::tempdir().unwrap();
        // Un chemin qui n'existe nulle part : seul le dernier segment compte.
        assert_eq!(
            nom_de_depart(
                &tmp.path()
                    .join("collection_2026-08-31.csv")
                    .display()
                    .to_string()
            ),
            "collection_2026-08-31.csv"
        );
        // Un dossier n'est pas un nom de fichier.
        assert_eq!(
            nom_de_depart(&tmp.path().display().to_string()),
            "collection.csv"
        );
        assert_eq!(nom_de_depart(""), "collection.csv");
        assert_eq!(nom_de_depart("   "), "collection.csv");
    }

    /// Deux clics sur « Parcourir… » n'ouvrent qu'une boîte.
    ///
    /// Deux dialogues modaux concurrents, c'est un clic perdu et un fil de
    /// plus — et le second écraserait le récepteur du premier, dont la
    /// réponse serait alors perdue en silence.
    #[test]
    fn une_seule_boite_a_la_fois() {
        let (_tmp, paths) = paths();
        let mut p = PanneauCsv::nouveau("essai", &paths);
        let (envoi, reception) = std::sync::mpsc::channel();
        p.dialogue = Some(Dialogue {
            cible: Face::Import,
            reception,
        });

        p.ouvrir_dialogue(Face::Export);
        assert_eq!(
            p.dialogue.as_ref().map(|d| d.cible),
            Some(Face::Import),
            "la boîte en cours n'a pas été remplacée"
        );

        // Et la réponse de la première arrive bien à destination.
        envoi.send(Some(PathBuf::from("/tmp/c.csv"))).unwrap();
        assert_eq!(
            p.relever_dialogue(),
            Some((Face::Import, PathBuf::from("/tmp/c.csv")))
        );
        assert!(p.dialogue.is_none(), "la boîte est refermée");
    }

    /// Une annulation ne touche à rien : l'utilisateur retrouve ce qu'il
    /// avait écrit.
    #[test]
    fn une_annulation_ne_change_aucun_champ() {
        let (_tmp, paths) = paths();
        let mut p = PanneauCsv::nouveau("essai", &paths);
        p.chemin_import = "C:\\garde-moi.csv".to_owned();
        let export_avant = p.chemin_export.clone();

        let (envoi, reception) = std::sync::mpsc::channel();
        p.dialogue = Some(Dialogue {
            cible: Face::Import,
            reception,
        });
        // Tant que rien n'arrive, la boîte reste ouverte.
        assert_eq!(p.relever_dialogue(), None);
        assert!(p.dialogue.is_some(), "la boîte est toujours à l'écran");

        envoi.send(None).unwrap();
        assert_eq!(p.relever_dialogue(), None, "annulé : aucun chemin");
        assert!(p.dialogue.is_none());
        assert_eq!(p.chemin_import, "C:\\garde-moi.csv");
        assert_eq!(p.chemin_export, export_avant);
    }

    /// Un fil mort sans réponse ne laisse pas le panneau en attente
    /// perpétuelle — sans quoi « Parcourir… » deviendrait inerte pour le
    /// reste de la session.
    #[test]
    fn un_fil_mort_libere_le_panneau() {
        let (_tmp, paths) = paths();
        let mut p = PanneauCsv::nouveau("essai", &paths);
        let (envoi, reception) = std::sync::mpsc::channel::<Option<PathBuf>>();
        p.dialogue = Some(Dialogue {
            cible: Face::Export,
            reception,
        });
        drop(envoi);
        assert_eq!(p.relever_dialogue(), None);
        assert!(p.dialogue.is_none(), "le panneau accepte un nouveau clic");
    }

    /// La destination proposée vit dans `export/` et porte la date.
    #[test]
    fn la_destination_par_defaut_est_datee() {
        let (_tmp, paths) = paths();
        let d = destination_par_defaut(&paths);
        assert!(d.ends_with(".csv"), "{d}");
        assert!(d.contains("collection_"), "{d}");
        assert!(
            d.starts_with(&paths.export().display().to_string()),
            "elle vit dans export/ : {d}"
        );
        let annee = chrono::Local::now().format("%Y").to_string();
        assert!(d.contains(&annee), "{d}");
    }

    /// Le message d'export dit les deux choses qui feront échouer un
    /// import chez Scanflip.
    #[test]
    fn le_message_d_export_avertit_de_ce_qui_sera_rejete() {
        let chemin = std::path::Path::new("/tmp/c.csv");

        let propre = ygo_app::export::Bilan {
            lignes: 550,
            exemplaires: 554,
            sans_etat: 0,
            ecartes: Vec::new(),
            raretes_refusees: Vec::new(),
        };
        let m = message_export(&propre, chemin);
        assert!(m.contains("550 ligne(s)"), "{m}");
        assert!(!m.contains("ATTENTION"), "rien à signaler : {m}");
        assert!(!m.contains("sans état"), "{m}");

        let bancal = ygo_app::export::Bilan {
            lignes: 10,
            exemplaires: 10,
            sans_etat: 3,
            ecartes: vec!["LOCR-JP".to_owned()],
            raretes_refusees: vec!["GMR".to_owned()],
        };
        let m = message_export(&bancal, chemin);
        assert!(m.contains("ATTENTION"), "{m}");
        assert!(m.contains("GMR"), "elle nomme la rareté fautive : {m}");
        assert!(m.contains("3 ligne(s) sans état"), "{m}");
    }

    /// Un dépôt sans CSV ne réveille pas le panneau ; un dépôt avec CSV
    /// l'ouvre sur l'import, fichier renseigné.
    ///
    /// # Ce test a d'abord menti
    ///
    /// Il réimplémentait la règle de l'extension dans une fermeture locale
    /// et l'affirmait sur elle-même — deux mutations du vrai code y
    /// survivaient tranquillement. La règle est maintenant dans `capter`,
    /// que le test appelle pour de bon.
    #[test]
    fn seul_un_csv_ouvre_le_panneau() {
        let (_tmp, paths) = paths();
        let mut p = PanneauCsv::nouveau("essai", &paths);

        // Rien qui ressemble à un CSV : le panneau ne bouge pas.
        let sans = [
            PathBuf::from("image.png"),
            PathBuf::from("sans_extension"),
            PathBuf::from("archive.zip"),
        ];
        assert!(!p.capter(&sans, &paths));
        assert_eq!(p.face(), Face::Fermee);
        assert!(p.chemin_import.is_empty());

        // Un CSV au milieu du lot suffit, quelle que soit la casse.
        let avec = [
            PathBuf::from("image.png"),
            PathBuf::from("/tmp/collection.CSV"),
        ];
        assert!(p.capter(&avec, &paths), "il dit avoir capté");
        assert_eq!(p.face(), Face::Import, "et s'ouvre sur l'import");
        assert!(
            p.chemin_import.ends_with("collection.CSV"),
            "{}",
            p.chemin_import
        );

        // Un lot vide ne fait rien non plus.
        let mut neuf = PanneauCsv::nouveau("essai", &paths);
        assert!(!neuf.capter(&[], &paths));
        assert_eq!(neuf.face(), Face::Fermee);
    }

    /// Un retour vide se reconnaît — l'appelant n'a alors rien à faire.
    #[test]
    fn un_retour_sans_rien_se_reconnait() {
        assert!(Retour::default().vide());
        assert!(!Retour {
            importe: true,
            ..Retour::default()
        }
        .vide());
        assert!(!Retour {
            creations: vec!["RA02".to_owned()],
            ..Retour::default()
        }
        .vide());
        assert!(!Retour {
            message: Some(("bonjour".to_owned(), false)),
            ..Retour::default()
        }
        .vide());
    }
}
