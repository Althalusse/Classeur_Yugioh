// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! L'écran d'accueil : la grille des classeurs.
//!
//! Toute la matière vient de [`ygo_app::accueil`], éprouvé sur les 26
//! classeurs réels des deux installations. Ici, il ne reste que les pixels —
//! et une seule règle, [`inscrire`], qui est pure et testée.
//!
//! # Le défaut des couvertures paysage
//!
//! Le prototype d'arbitrage inscrivait la couverture avec
//! `fit_to_exact_size` à l'intérieur d'un `allocate_ui_with_layout`. Pour une
//! image portrait — huit des neuf boosters — l'image remplit la hauteur et
//! tout va bien. Pour `LDK2`, qui est en **paysage** (997 × 692, ratio 1,44),
//! l'image ne fait plus que 153 points de haut dans une zone qui en réserve
//! 230 : le `Ui` ne consomme que la hauteur réellement employée, et **tout le
//! texte de la tuile remonte de 77 points**. Une tuile sur neuf n'était pas
//! alignée avec ses voisines.
//!
//! La correction tient en deux temps : la zone est réservée en entier par
//! `allocate_exact_size`, puis l'image est peinte dans le rectangle que
//! [`inscrire`] calcule, centré dedans. La géométrie ne dépend plus de ce que
//! l'image mesure, et `paint_at` — qui déforme quand on lui donne un
//! rectangle au mauvais ratio — reçoit ici un rectangle au **bon** ratio par
//! construction.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::csv::{Face, PanneauCsv};
use eframe::egui;
use ygo_app::accueil::{self, Classeur, Ordre, OrigineCouverture};
use ygo_core::config::Config;
use ygo_core::paths::Paths;

/// Largeur d'une tuile, en points.
const LARGEUR_TUILE: f32 = 220.0;
/// Hauteur réservée à la couverture — **toujours** consommée en entier.
const HAUTEUR_COUVERTURE: f32 = 230.0;
/// Hauteur totale d'une tuile, fixe, pour que les rangées s'alignent.
const HAUTEUR_TUILE: f32 = 336.0;
/// Espacement entre tuiles.
const ESPACE: f32 = 12.0;

/// La taille à donner à une image pour l'inscrire dans une zone **sans la
/// déformer**.
///
/// Le facteur d'échelle est le plus petit des deux rapports : l'image touche
/// la zone par sa dimension contraignante et laisse du vide sur l'autre. Elle
/// est agrandie si elle est plus petite que la zone — une couverture de 338
/// points de large doit remplir la tuile comme les autres, sinon la grille a
/// l'air trouée.
///
/// Une dimension nulle, d'un côté ou de l'autre, rend une taille nulle : une
/// image que le décodeur n'a pas su mesurer ne doit pas produire un rectangle
/// infini.
///
/// ```
/// use ygo_ui::accueil::inscrire;
/// // Portrait dans une zone portrait : la hauteur contraint.
/// assert_eq!(inscrire((100, 200), (220.0, 230.0)), (115.0, 230.0));
/// // Paysage : c'est la largeur qui contraint, et il reste du vide en haut
/// // et en bas — que la tuile réserve quand même.
/// assert_eq!(inscrire((200, 100), (220.0, 230.0)), (220.0, 110.0));
/// ```
#[must_use]
pub fn inscrire(image: (u32, u32), zone: (f32, f32)) -> (f32, f32) {
    let (largeur, hauteur) = image;
    if largeur == 0 || hauteur == 0 || zone.0 <= 0.0 || zone.1 <= 0.0 {
        return (0.0, 0.0);
    }
    #[allow(clippy::cast_precision_loss)]
    let (l, h) = (largeur as f32, hauteur as f32);
    let echelle = (zone.0 / l).min(zone.1 / h);
    (l * echelle, h * echelle)
}

/// Une couverture chargée : ses octets, et ce qu'elle mesure.
///
/// La taille est lue à part, par `image::image_dimensions`, qui ne décode que
/// l'en-tête. C'est elle qui permet à [`inscrire`] de faire son calcul avant
/// qu'egui n'ait décodé quoi que ce soit — sans quoi la première image
/// s'afficherait avec une géométrie, et la seconde avec une autre.
struct Couverture {
    octets: Arc<[u8]>,
    taille: (u32, u32),
}

/// L'écran d'accueil.
pub struct EcranAccueil {
    racine: PathBuf,
    paths: Paths,
    grille_defaut: (u8, u8),

    classeurs: Vec<Classeur>,
    couvertures: HashMap<String, Couverture>,
    total: usize,
    possedees: usize,

    francais: bool,
    filtre: String,
    ordre: Ordre,

    /// Le classeur qu'un clic demande à ouvrir. Lu et repris par l'application,
    /// jamais par l'écran lui-même : c'est ce qui garde la navigation hors
    /// d'ici.
    ouvrir: Option<String>,
    /// Un passage aux options a été demandé.
    options: bool,
    /// La création d'un classeur a été demandée.
    creation: bool,
    /// Le classeur dont la suppression est en cours de confirmation.
    ///
    /// La conséquence est calculée **une fois**, à l'ouverture de la boîte :
    /// la recalculer à chaque image rouvrirait la base soixante fois par
    /// seconde.
    a_supprimer: Option<ygo_app::suppression::Consequence>,
    /// La case « j'ai compris » d'une suppression définitive qui détruit des
    /// quantités saisies. Remise à zéro à chaque ouverture de la boîte.
    perte_acceptee: bool,
    /// Une suppression confirmée, à exécuter par l'application.
    supprimer: Option<(String, ygo_app::suppression::Mode)>,
    /// La bascule de masse en cours de confirmation : le classeur, ce qu'elle
    /// ferait, et l'état lu **une fois** à l'ouverture de la boîte.
    a_basculer: Option<(
        String,
        ygo_app::possession::Bascule,
        ygo_app::possession::Etat,
    )>,
    /// Ce que la corbeille contient, compté à la lecture.
    ///
    /// Recompté à chaque `recharger`, jamais à chaque image : c'est un
    /// listage de dossier, et il n'a rien à faire dans une boucle de rendu.
    corbeille: usize,
    /// Un passage à la corbeille a été demandé.
    voir_corbeille: bool,
    /// Les artworks alternatifs d'un classeur, ou de toute l'installation.
    voir_artworks: Option<Option<String>>,
    /// Un passage à l'inventaire a été demandé.
    inventaire: bool,
    /// Un passage aux statistiques a été demandé.
    stats: bool,
    /// L'import/export, le même que celui de l'inventaire.
    ///
    /// # Pourquoi ici aussi
    ///
    /// Il ne vivait que dans l'écran de l'inventaire, derrière un bouton
    /// nommé « Toutes mes cartes » : rien sur l'accueil ne laissait deviner
    /// que l'application savait importer. Le panneau est un composant
    /// partagé, pas une copie — une seule implémentation, deux endroits où
    /// l'atteindre.
    csv: PanneauCsv,
    /// Les classeurs dont la création a été demandée depuis l'import.
    creations_csv: Vec<String>,
    /// Ce que le panneau a fait, à afficher.
    message_csv: Option<(String, bool)>,

    duree_lecture: Duration,
    duree_couvertures: Duration,
    depart: Instant,
    premier_rendu: Option<Duration>,
}

impl std::fmt::Debug for EcranAccueil {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EcranAccueil")
            .field("classeurs", &self.classeurs.len())
            .field("total", &self.total)
            .field("possedees", &self.possedees)
            .finish()
    }
}

impl EcranAccueil {
    /// Lit l'installation et prépare l'écran.
    #[must_use]
    pub fn ouvrir(racine: PathBuf) -> Self {
        let depart = Instant::now();
        let paths = Paths::depuis_racine(&racine);
        let config = Config::charger(paths.app_config());
        let grille_defaut = config.grille_defaut();
        let francais = config.langue().code() == "FR";

        let paths_corbeille = paths.clone();

        let lecture = Instant::now();
        let classeurs = accueil::lister(&paths, grille_defaut);
        let duree_lecture = lecture.elapsed();
        let (total, possedees) = accueil::totaux(&classeurs);

        let images = Instant::now();
        let couvertures = charger_couvertures(&classeurs);
        let duree_couvertures = images.elapsed();

        println!(
            "accueil — {} classeurs lus en {duree_lecture:?}, {} couverture(s) en {duree_couvertures:?}",
            classeurs.len(),
            couvertures.len()
        );

        let mut ecran = Self {
            racine,
            paths,
            grille_defaut,
            classeurs,
            couvertures,
            total,
            possedees,
            francais,
            filtre: String::new(),
            ordre: Ordre::default(),
            ouvrir: None,
            options: false,
            creation: false,
            a_supprimer: None,
            perte_acceptee: false,
            supprimer: None,
            a_basculer: None,
            corbeille: ygo_app::suppression::contenu_corbeille(&paths_corbeille).len(),
            voir_corbeille: false,
            voir_artworks: None,
            inventaire: false,
            stats: false,
            csv: PanneauCsv::nouveau("accueil", &paths_corbeille),
            creations_csv: Vec::new(),
            message_csv: None,
            duree_lecture,
            duree_couvertures,
            depart,
            premier_rendu: None,
        };
        accueil::trier(&mut ecran.classeurs, ecran.ordre);
        ecran
    }

    /// La racine de l'installation, pour ouvrir un classeur.
    #[must_use]
    pub fn racine(&self) -> &PathBuf {
        &self.racine
    }

    /// Reprend la demande d'ouverture, s'il y en a une.
    ///
    /// L'appelant la consomme : deux lectures d'affilée ne rouvrent pas deux
    /// fois le même classeur.
    pub fn demande_ouverture(&mut self) -> Option<String> {
        self.ouvrir.take()
    }

    /// Reprend la demande d'ouverture des options.
    pub fn demande_options(&mut self) -> bool {
        std::mem::take(&mut self.options)
    }

    /// Reprend la demande de création d'un classeur.
    pub fn demande_creation(&mut self) -> bool {
        std::mem::take(&mut self.creation)
    }

    /// Reprend la demande d'ouverture des statistiques.
    pub fn demande_stats(&mut self) -> bool {
        std::mem::take(&mut self.stats)
    }

    /// Reprend les créations de classeur demandées depuis l'import.
    pub fn creations_csv_demandees(&mut self) -> Vec<String> {
        std::mem::take(&mut self.creations_csv)
    }

    /// Reprend la demande d'ouverture de l'inventaire.
    pub fn demande_inventaire(&mut self) -> bool {
        std::mem::take(&mut self.inventaire)
    }

    /// Reprend la demande d'ouverture de la corbeille.
    pub fn demande_corbeille(&mut self) -> bool {
        std::mem::take(&mut self.voir_corbeille)
    }

    /// Reprend la demande d'ouverture des artworks alternatifs.
    ///
    /// `Some(Some(code))` pour un classeur, `Some(None)` pour toute
    /// l'installation, `None` si rien n'a été demandé.
    pub fn demande_artworks(&mut self) -> Option<Option<String>> {
        std::mem::take(&mut self.voir_artworks)
    }

    /// Combien d'entrées la corbeille contient, telle qu'elle a été lue.
    #[must_use]
    pub fn corbeille(&self) -> usize {
        self.corbeille
    }

    /// Reprend la suppression **confirmée** d'un classeur, et son mode.
    ///
    /// Rendue seulement après confirmation : l'écran ne demande jamais une
    /// suppression que l'utilisateur n'a pas approuvée dans la boîte.
    pub fn suppression_confirmee(&mut self) -> Option<(String, ygo_app::suppression::Mode)> {
        self.supprimer.take()
    }

    /// Les chemins de l'installation.
    #[must_use]
    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    /// Relit les classeurs **et leurs couvertures**.
    ///
    /// À appeler quand un classeur vient d'être créé, ou qu'une couverture
    /// vient d'être téléchargée : [`recharger`](Self::recharger) seul relirait
    /// les compteurs sans jamais aller chercher la nouvelle image.
    pub fn recharger_tout(&mut self) {
        self.recharger();
        let debut = Instant::now();
        self.couvertures = charger_couvertures(&self.classeurs);
        self.duree_couvertures = debut.elapsed();
    }

    /// Relit les classeurs — à faire au retour d'un classeur, dont les
    /// quantités ont pu changer.
    ///
    /// Les couvertures ne sont **pas** rechargées : elles ne bougent pas, et
    /// les relire coûterait le seul temps sensible de cet écran. Un classeur
    /// neuf, lui, en a une à découvrir — c'est [`recharger_tout`](Self::recharger_tout).
    pub fn recharger(&mut self) {
        let lecture = Instant::now();
        self.classeurs = accueil::lister(&self.paths, self.grille_defaut);
        self.duree_lecture = lecture.elapsed();
        let (total, possedees) = accueil::totaux(&self.classeurs);
        self.total = total;
        self.possedees = possedees;
        // Une suppression vient d'y déposer un classeur, une restauration
        // vient d'en retirer un : le bouton doit suivre.
        self.corbeille = ygo_app::suppression::contenu_corbeille(&self.paths).len();
        accueil::trier(&mut self.classeurs, self.ordre);
    }
}

/// Lit les octets et les dimensions de chaque couverture.
///
/// Les images sont passées à egui en **octets**, pas en URI `file://` : sous
/// Windows, `file://H:\…` donne deux barres au lieu de trois et des
/// antislashs, où egui ne voit qu'un hôte réseau inconnu — dix-sept triangles
/// d'erreur au premier essai.
fn charger_couvertures(classeurs: &[Classeur]) -> HashMap<String, Couverture> {
    classeurs
        .iter()
        .filter_map(|c| {
            let chemin = c.couverture.as_ref()?;
            let octets = std::fs::read(chemin).ok()?;
            // `image_dimensions` ne lit que l'en-tête.
            let taille = image::image_dimensions(chemin).unwrap_or((0, 0));
            Some((
                c.code.clone(),
                Couverture {
                    octets: Arc::from(octets.into_boxed_slice()),
                    taille,
                },
            ))
        })
        .collect()
}

impl eframe::App for EcranAccueil {
    fn ui(&mut self, racine: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.premier_rendu.is_none() {
            let ecoule = self.depart.elapsed();
            self.premier_rendu = Some(ecoule);
            println!("accueil — premier rendu à {ecoule:?}");
        }
        let paths = self.paths.clone();
        self.csv.capter_depot(racine.ctx(), &paths);
        self.barre_du_haut(racine);
        self.barre_du_bas(racine);
        let retour = self.csv.afficher(racine, &paths, self.francais);
        if !retour.vide() {
            if let Some(m) = retour.message {
                self.message_csv = Some(m);
            }
            if !retour.creations.is_empty() {
                self.creations_csv = retour.creations;
            }
            // Un import change les quantités de plusieurs classeurs : les
            // compteurs de chaque tuile sont à relire.
            if retour.importe {
                self.recharger();
            }
        }
        self.grille(racine);
        self.confirmation_suppression(racine.ctx());
        self.confirmation_bascule(racine.ctx());
    }
}

impl EcranAccueil {
    /// Les bascules de masse, confirmées avant d'écrire.
    ///
    /// # Pourquoi confirmer « Tout possédé » aussi
    ///
    /// Cocher 698 lignes ne détruit rien en soi — mais l'annuler demande de
    /// tout décocher, ce qui emporte alors les quantités qu'on avait
    /// vraiment saisies. Les deux sens se confirment donc, et la boîte dit
    /// combien de lignes changeraient.
    fn confirmation_bascule(&mut self, ctx: &egui::Context) {
        use ygo_app::possession::Bascule;
        let Some((code, bascule, etat)) = self.a_basculer.clone() else {
            return;
        };
        let mut ouverte = true;
        let mut confirme = false;
        egui::Window::new(format!("{} — {code} ?", bascule.libelle()))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .open(&mut ouverte)
            .show(ctx, |ui| {
                ui.set_min_width(440.0);
                ui.add_space(4.0);
                match bascule {
                    Bascule::ToutPosseder => {
                        ui.label(format!(
                            "{} ligne(s) passeraient à ×1. Les {} déjà possédée(s) \
                             gardent leur quantité.",
                            etat.lignes.saturating_sub(etat.possedees),
                            etat.possedees
                        ));
                        ui.small("L'état et l'édition ne sont pas touchés.");
                    }
                    Bascule::ToutRemettreAZero => {
                        ui.colored_label(
                            ui.visuals().warn_fg_color,
                            format!(
                                "{} ligne(s) possédée(s), {} exemplaire(s) — \
                                 ils n'existent nulle part ailleurs.",
                                etat.possedees, etat.exemplaires
                            ),
                        );
                        ui.add_space(4.0);
                        ui.small(
                            "L'état et l'édition partent avec : ils décrivent des \
                             exemplaires. Les cartes du classeur, elles, restent.",
                        );
                    }
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button("Annuler").clicked() {
                        self.a_basculer = None;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(bascule.libelle()).clicked() {
                            confirme = true;
                        }
                    });
                });
                ui.add_space(4.0);
            });
        if confirme {
            match ygo_app::possession::appliquer_au_classeur(&self.paths, &code, bascule) {
                Ok(touchees) => {
                    self.message_csv =
                        Some((format!("{code} — {} ligne(s) modifiée(s)", touchees), false));
                    self.recharger();
                }
                Err(e) => self.message_csv = Some((e.to_string(), true)),
            }
            self.a_basculer = None;
        } else if !ouverte {
            self.a_basculer = None;
        }
    }

    /// La boîte qui demande confirmation avant de retirer un classeur.
    ///
    /// Elle **nomme ce qui se perd** : le nombre d'exemplaires possédés, qui
    /// n'existent nulle part ailleurs que dans cette base. Un classeur vide le
    /// dit aussi — inutile d'alarmer pour rien.
    ///
    /// # Deux sorties, parce que la question se pose ici — 2026-09-20
    ///
    /// Le Python n'offrait que la corbeille, puis un second passage par
    /// l'écran de la corbeille pour effacer. Deux gestes séparés dans le temps
    /// pour un classeur dont on sait déjà, en cliquant « Supprimer », qu'on
    /// n'en veut plus. L'utilisateur l'a demandé, et il a raison : le choix
    /// appartient au moment où l'on décide, pas à un écran qu'il faudra
    /// rouvrir.
    ///
    /// La corbeille reste le bouton de droite — celui que la main trouve, et
    /// celui qu'on peut défaire. « Supprimer définitivement » est à sa gauche,
    /// et **quand il y a des quantités saisies, il est verrouillé derrière une
    /// case à cocher** : un classeur vide s'efface d'un clic, un classeur
    /// peuplé demande qu'on dise qu'on a lu.
    fn confirmation_suppression(&mut self, ctx: &egui::Context) {
        use ygo_app::suppression::Mode;

        let Some(consequence) = self.a_supprimer.clone() else {
            return;
        };
        let mut ouverte = true;
        let mut choisi: Option<Mode> = None;
        egui::Window::new(format!("Supprimer {} ?", consequence.code))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .open(&mut ouverte)
            .show(ctx, |ui| {
                ui.set_min_width(460.0);
                ui.add_space(4.0);
                if consequence.irreversible() {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        format!(
                            "{} exemplaire(s) possédé(s) sur {} carte(s) — \
                             ils n'existent nulle part ailleurs.",
                            consequence.exemplaires, consequence.lignes_possedees
                        ),
                    );
                } else {
                    ui.label(format!(
                        "{} ligne(s), aucune carte possédée.",
                        consequence.lignes
                    ));
                }
                ui.add_space(10.0);

                // Les deux issues, dites avant d'être cliquées : le bouton
                // seul ne dit pas ce qu'il fait du dossier.
                ui.label(egui::RichText::new(Mode::Corbeille.consequence()).weak());
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(Mode::Definitif.consequence())
                        .weak()
                        .color(ui.visuals().warn_fg_color),
                );
                ui.add_space(8.0);
                ui.small("Les images de cartes et la couverture ne sont pas touchées.");

                // La case n'apparaît que s'il y a quelque chose à perdre.
                // L'afficher sur un classeur vide, ce serait demander de
                // confirmer un risque qui n'existe pas — et apprendre à
                // cocher sans lire.
                if consequence.irreversible() {
                    ui.add_space(8.0);
                    ui.checkbox(
                        &mut self.perte_acceptee,
                        format!(
                            "Je veux perdre les {} exemplaire(s) saisi(s)",
                            consequence.exemplaires
                        ),
                    );
                }

                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button("Annuler").clicked() {
                        self.a_supprimer = None;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(Mode::Corbeille.libelle()).clicked() {
                            choisi = Some(Mode::Corbeille);
                        }
                        ui.add_space(8.0);
                        let permis = !consequence.irreversible() || self.perte_acceptee;
                        let bouton = ui.add_enabled(
                            permis,
                            egui::Button::new(
                                egui::RichText::new(Mode::Definitif.libelle())
                                    .color(ui.visuals().error_fg_color),
                            ),
                        );
                        if bouton.clicked() {
                            choisi = Some(Mode::Definitif);
                        }
                        if !permis {
                            bouton.on_disabled_hover_text(
                                "Cochez la case : ce classeur porte des quantités saisies",
                            );
                        }
                    });
                });
                ui.add_space(4.0);
            });
        if let Some(mode) = choisi {
            self.supprimer = Some((consequence.code, mode));
            self.a_supprimer = None;
        } else if !ouverte {
            self.a_supprimer = None;
        }
    }

    fn barre_du_haut(&mut self, racine: &mut egui::Ui) {
        egui::Panel::top("entete-accueil").show(racine, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.heading("Mes classeurs");

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("⚙ Options").clicked() {
                        self.options = true;
                    }
                    ui.add_space(8.0);
                    if ui.button("+ Nouveau classeur").clicked() {
                        self.creation = true;
                    }
                    ui.add_space(8.0);
                    if ui
                        .button("Statistiques")
                        .on_hover_text("Où en est chaque classeur, et sur quelles raretés")
                        .clicked()
                    {
                        self.stats = true;
                    }
                    ui.add_space(8.0);
                    if ui.button("Exporter…").clicked() {
                        self.csv.basculer(Face::Export);
                    }
                    ui.add_space(8.0);
                    if ui
                        .button("Importer…")
                        .on_hover_text(
                            "Un CSV Scanflip — vous pouvez aussi le déposer \
                             directement sur la fenêtre",
                        )
                        .clicked()
                    {
                        self.csv.basculer(Face::Import);
                    }
                    ui.add_space(8.0);
                    // L'inventaire est la vue transverse : un bouton fixe,
                    // parce qu'il est utile quel que soit l'état de la
                    // collection — contrairement à la corbeille.
                    if ui
                        .button("Toutes mes cartes")
                        .on_hover_text(
                            "Les cartes possédées, tous classeurs confondus —                              filtres, état en masse, import et export",
                        )
                        .clicked()
                    {
                        self.inventaire = true;
                    }
                    // Le bouton ne paraît que si la corbeille contient
                    // quelque chose, et il dit combien : c'est le seul moment
                    // où le chiffre renseigne.
                    if self.corbeille > 0 {
                        ui.add_space(8.0);
                        if ui
                            .button(format!("🗑 Corbeille ({})", self.corbeille))
                            .on_hover_text(
                                "Voir les classeurs supprimés, les remettre ou les effacer",
                            )
                            .clicked()
                        {
                            self.voir_corbeille = true;
                        }
                    }
                    ui.add_space(8.0);
                    if ui.checkbox(&mut self.francais, "FR").changed() {
                        // Le filtre porte sur le nom affiché : changer de
                        // langue change ce qu'il retient.
                    }
                    ui.add_space(8.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.filtre)
                            .hint_text("filtrer…")
                            .desired_width(180.0),
                    );
                    ui.add_space(8.0);
                    let mut ordre = self.ordre;
                    egui::ComboBox::from_id_salt("ordre-accueil")
                        .selected_text(ordre.libelle())
                        .show_ui(ui, |ui| {
                            for o in Ordre::tous() {
                                ui.selectable_value(&mut ordre, o, o.libelle());
                            }
                        });
                    if ordre != self.ordre {
                        self.ordre = ordre;
                        accueil::trier(&mut self.classeurs, ordre);
                    }
                    ui.label("Trier par");
                });
            });
            // Les compteurs ont leur propre ligne.
            //
            // Ils partageaient celle du titre, à gauche des commandes
            // alignées à droite. Sur une fenêtre où la somme des deux
            // dépassait la largeur, egui ne rétrécit pas : il superpose.
            // « 550 possédée(s) (33,8 %) » s'écrivait par-dessus « Trier
            // par », et les deux devenaient illisibles.
            ui.add_space(2.0);
            ui.label(format!(
                "{} classeur(s) · {} cartes · {} possédée(s) ({:.1} %)",
                self.classeurs.len(),
                self.total,
                self.possedees,
                part(self.possedees, self.total)
            ));
            if let Some((texte, erreur)) = self.message_csv.clone() {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let couleur = if erreur {
                        ui.visuals().error_fg_color
                    } else {
                        ui.visuals().hyperlink_color
                    };
                    ui.colored_label(couleur, texte);
                    if ui.small_button("✖").clicked() {
                        self.message_csv = None;
                    }
                });
            }
            ui.add_space(8.0);
        });
    }

    fn barre_du_bas(&mut self, racine: &mut egui::Ui) {
        egui::Panel::bottom("mesures-accueil").show(racine, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.small(format!("{}", self.racine.display()));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.small(format!(
                        "lecture {:?} · couvertures {:?}{}",
                        self.duree_lecture,
                        self.duree_couvertures,
                        match self.premier_rendu {
                            Some(t) => format!(" · premier rendu {t:?}"),
                            None => String::new(),
                        }
                    ));
                });
            });
            ui.add_space(4.0);
        });
    }

    fn grille(&mut self, racine: &mut egui::Ui) {
        egui::CentralPanel::default().show(racine, |ui| {
            let visibles: Vec<Classeur> =
                accueil::filtrer(&self.classeurs, &self.filtre, self.francais)
                    .into_iter()
                    .cloned()
                    .collect();

            if visibles.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(96.0);
                    if self.classeurs.is_empty() {
                        ui.heading("Aucun classeur dans cette installation.");
                        ui.add_space(8.0);
                        if ui.button("+ Créer un classeur").clicked() {
                            self.creation = true;
                        }
                    } else {
                        ui.heading(format!(
                            "Aucun classeur ne correspond à « {} ».",
                            self.filtre.trim()
                        ));
                        ui.add_space(8.0);
                        if ui.button("Effacer le filtre").clicked() {
                            self.filtre.clear();
                        }
                    }
                });
                return;
            }

            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let par_ligne = (((ui.available_width() - ESPACE) / (LARGEUR_TUILE + ESPACE)).floor()
                as usize)
                .max(1);

            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(ESPACE);
                for tranche in visibles.chunks(par_ligne) {
                    ui.horizontal_top(|ui| {
                        ui.add_space(ESPACE);
                        for classeur in tranche {
                            let reponse = self.tuile(ui, classeur);
                            if reponse.clicked() {
                                self.ouvrir = Some(classeur.code.clone());
                            }
                            // Le clic droit, comme dans le classeur : la
                            // suppression n'a pas de bouton visible, elle se
                            // demande. Un bouton « ✕ » sur chaque tuile
                            // s'attraperait du coin de la souris.
                            reponse.context_menu(|ui| {
                                ui.label(egui::RichText::new(&classeur.code).strong());
                                ui.separator();
                                if ui.button("Ouvrir").clicked() {
                                    self.ouvrir = Some(classeur.code.clone());
                                    ui.close();
                                }
                                ui.separator();
                                // Les deux bascules de masse. L'état est lu
                                // ici, à l'ouverture du menu, et pas à chaque
                                // image : c'est une requête par classeur.
                                let etat = ygo_app::possession::etat_du_classeur(
                                    &self.paths,
                                    &classeur.code,
                                );
                                for (bascule, actif) in [
                                    (ygo_app::possession::Bascule::ToutPosseder, etat.incomplet()),
                                    (
                                        ygo_app::possession::Bascule::ToutRemettreAZero,
                                        etat.a_quelque_chose(),
                                    ),
                                ] {
                                    let bouton = ui.add_enabled(
                                        actif,
                                        egui::Button::new(format!("{}…", bascule.libelle())),
                                    );
                                    if bouton.clicked() {
                                        self.a_basculer =
                                            Some((classeur.code.clone(), bascule, etat));
                                        ui.close();
                                    }
                                    if !actif {
                                        bouton.on_disabled_hover_text(match bascule {
                                            ygo_app::possession::Bascule::ToutPosseder => {
                                                "Tout est déjà possédé"
                                            }
                                            ygo_app::possession::Bascule::ToutRemettreAZero => {
                                                "Rien n'est possédé"
                                            }
                                        });
                                    }
                                }
                                // « Artworks alternatifs… » se lisait comme une
                                // action sur la **couverture** : le menu est
                                // celui de la tuile, et la tuile, à l'œil,
                                // c'est la cover. Le mot « cartes » lève
                                // l'ambiguïté sans rien changer d'autre.
                                if ui
                                    .button("Artworks des cartes…")
                                    .on_hover_text(
                                        "Les illustrations alternatives des cartes de ce \
                                         classeur — la couverture n'est pas concernée",
                                    )
                                    .clicked()
                                {
                                    self.voir_artworks = Some(Some(classeur.code.clone()));
                                    ui.close();
                                }
                                ui.separator();
                                if ui.button("Supprimer…").clicked() {
                                    self.perte_acceptee = false;
                                    self.a_supprimer = Some(ygo_app::suppression::consequence(
                                        &self.paths,
                                        &classeur.code,
                                    ));
                                    ui.close();
                                }
                            });
                        }
                    });
                    ui.add_space(ESPACE);
                }
            });
        });
    }

    /// Une tuile : couverture, nom, compteurs, jauge. Cliquable en entier.
    fn tuile(&self, ui: &mut egui::Ui, classeur: &Classeur) -> egui::Response {
        let reponse = egui::Frame::group(ui.style())
            .corner_radius(8.0)
            .inner_margin(10.0)
            .show(ui, |ui| {
                ui.set_width(LARGEUR_TUILE);
                ui.set_height(HAUTEUR_TUILE);
                ui.vertical(|ui| {
                    self.couverture(ui, classeur);

                    ui.add_space(6.0);
                    ui.label(egui::RichText::new(classeur.code.as_str()).strong());
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(classeur.nom_affiche(self.francais))
                                .small()
                                .weak(),
                        )
                        .truncate(),
                    );

                    ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                        ui.small(
                            egui::RichText::new(format!(
                                "grille {}×{} · {}",
                                classeur.colonnes,
                                classeur.lignes,
                                origine(classeur.origine_couverture)
                            ))
                            .weak(),
                        );
                        ui.horizontal(|ui| {
                            ui.small(format!("{} / {}", classeur.possedees, classeur.total));
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.small(format!("{:.0} %", classeur.pourcentage()));
                                },
                            );
                        });
                        #[allow(clippy::cast_possible_truncation)]
                        ui.add(
                            egui::ProgressBar::new((classeur.pourcentage() / 100.0) as f32)
                                .desired_height(6.0)
                                .corner_radius(3.0),
                        );
                    });
                });
            })
            .response;

        let reponse = reponse.interact(egui::Sense::click());
        if reponse.hovered() {
            ui.painter().rect_stroke(
                reponse.rect,
                8.0,
                egui::Stroke::new(1.5, ui.visuals().selection.bg_fill),
                egui::StrokeKind::Inside,
            );
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        reponse.on_hover_text(format!(
            "Ouvrir {} — {}",
            classeur.code,
            classeur.nom_affiche(self.francais)
        ))
    }

    /// La couverture, inscrite sans déformation dans une zone de hauteur fixe.
    fn couverture(&self, ui: &mut egui::Ui, classeur: &Classeur) {
        let zone = egui::vec2(LARGEUR_TUILE, HAUTEUR_COUVERTURE);
        // La zone est réservée EN ENTIER, quoi que l'image mesure : c'est ce
        // qui garde les tuiles alignées quand l'une des couvertures est en
        // paysage.
        let (rect, _) = ui.allocate_exact_size(zone, egui::Sense::hover());

        let Some(couverture) = self.couvertures.get(&classeur.code) else {
            ui.painter()
                .rect_filled(rect, 4.0, ui.visuals().extreme_bg_color);
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "📂",
                egui::FontId::proportional(48.0),
                ui.visuals().weak_text_color(),
            );
            return;
        };

        let (largeur, hauteur) = inscrire(couverture.taille, (zone.x, zone.y));
        if largeur <= 0.0 || hauteur <= 0.0 {
            ui.painter()
                .rect_filled(rect, 4.0, ui.visuals().extreme_bg_color);
            return;
        }
        let inscrit = egui::Rect::from_center_size(rect.center(), egui::vec2(largeur, hauteur));
        egui::Image::from_bytes(
            format!("bytes://cover/{}", classeur.code),
            egui::load::Bytes::Shared(Arc::clone(&couverture.octets)),
        )
        .corner_radius(4.0)
        .paint_at(ui, inscrit);
    }
}

fn origine(o: OrigineCouverture) -> &'static str {
    match o {
        OrigineCouverture::Booster => "cover booster",
        OrigineCouverture::Carte => "cover carte",
        OrigineCouverture::Dossier => "cover dossier",
        OrigineCouverture::Aucune => "sans cover",
    }
}

fn part(numerateur: usize, denominateur: usize) -> f64 {
    if denominateur == 0 {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss)]
    {
        numerateur as f64 / denominateur as f64 * 100.0
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    const ZONE: (f32, f32) = (LARGEUR_TUILE, HAUTEUR_COUVERTURE);

    fn installation() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let racine = tmp.path().to_path_buf();
        std::fs::create_dir_all(Paths::depuis_racine(&racine).classeurs()).unwrap();
        (tmp, racine)
    }

    fn classeur(paths: &Paths, code: &str) {
        std::fs::create_dir_all(paths.dossier_classeur(code)).unwrap();
        let conn = ygo_db::rusqlite::Connection::open(paths.classeur_db(code)).unwrap();
        conn.execute(
            "CREATE TABLE cards (name TEXT, set_code TEXT, rarity TEXT, quantite INTEGER)",
            (),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO cards VALUES ('X', 'RA05-EN001', 'Rare', 1)",
            (),
        )
        .unwrap();
    }

    /// Les créations réclamées par l'import se consomment aussi.
    #[test]
    fn les_creations_de_l_import_se_consomment() {
        let (_tmp, racine) = installation();
        let mut ecran = EcranAccueil::ouvrir(racine);
        assert!(ecran.creations_csv_demandees().is_empty());
        ecran.creations_csv = vec!["BLTR".to_owned(), "RA03".to_owned()];
        assert_eq!(ecran.creations_csv_demandees().len(), 2);
        assert!(
            ecran.creations_csv_demandees().is_empty(),
            "une seule fois — sinon l'application les recréerait à chaque image"
        );
    }

    /// L'accueil et l'inventaire montrent le **même** panneau, pas deux
    /// copies : leurs identifiants diffèrent, leur comportement non.
    #[test]
    fn l_accueil_porte_son_propre_panneau_csv() {
        let (_tmp, racine) = installation();
        let mut ecran = EcranAccueil::ouvrir(racine);
        assert_eq!(ecran.csv.face(), crate::csv::Face::Fermee);
        ecran.csv.basculer(crate::csv::Face::Import);
        assert_eq!(ecran.csv.face(), crate::csv::Face::Import);
    }

    /// La demande de statistiques se consomme aussi.
    #[test]
    fn la_demande_de_statistiques_se_consomme() {
        let (_tmp, racine) = installation();
        let mut ecran = EcranAccueil::ouvrir(racine);
        ecran.stats = true;
        assert!(ecran.demande_stats());
        assert!(!ecran.demande_stats(), "une seule fois");
    }

    /// La demande d'inventaire se consomme, comme les autres.
    #[test]
    fn la_demande_d_inventaire_se_consomme() {
        let (_tmp, racine) = installation();
        let mut ecran = EcranAccueil::ouvrir(racine);
        ecran.inventaire = true;
        assert!(ecran.demande_inventaire());
        assert!(!ecran.demande_inventaire(), "une seule fois");
    }

    /// Les demandes se consomment : l'application ne rouvre pas la corbeille
    /// à chaque image parce qu'elle a été demandée une fois.
    #[test]
    fn la_demande_de_corbeille_se_consomme() {
        let (_tmp, racine) = installation();
        let mut ecran = EcranAccueil::ouvrir(racine);
        ecran.voir_corbeille = true;
        assert!(ecran.demande_corbeille());
        assert!(!ecran.demande_corbeille(), "une seule fois");
    }

    /// Le compte suit ce que la corbeille contient — et il le suit **au
    /// rechargement**, pas seulement à l'ouverture : c'est après une
    /// suppression que le bouton doit apparaître, sans quitter l'accueil.
    #[test]
    fn le_compte_de_la_corbeille_est_relu_au_rechargement() {
        let (_tmp, racine) = installation();
        let paths = Paths::depuis_racine(&racine);
        classeur(&paths, "RA05");

        let mut ecran = EcranAccueil::ouvrir(racine);
        assert_eq!(ecran.corbeille(), 0, "rien à l'ouverture");

        ygo_app::suppression::vers_corbeille(&paths, "RA05", "2026-08-29_120000").unwrap();
        ecran.recharger();
        assert_eq!(ecran.corbeille(), 1, "la suppression se voit sans quitter");

        ygo_app::suppression::restaurer(&paths, "RA05_2026-08-29_120000").unwrap();
        ecran.recharger();
        assert_eq!(ecran.corbeille(), 0, "et la restauration aussi");
    }

    /// Le ratio est conservé, dans les deux orientations. C'est la propriété
    /// que `paint_at` seul ne garantissait pas — et le défaut « cartes
    /// aplaties » que le Python avait mis six cycles à corriger.
    #[test]
    fn le_ratio_est_conserve() {
        for image in [(997, 692), (338, 614), (511, 902), (1, 3000), (4000, 1)] {
            let (l, h) = inscrire(image, ZONE);
            #[allow(clippy::cast_precision_loss)]
            let attendu = image.0 as f32 / image.1 as f32;
            assert!(
                (l / h - attendu).abs() < 1e-3,
                "{image:?} → {l}×{h}, ratio {} au lieu de {attendu}",
                l / h
            );
        }
    }

    /// L'image tient dans la zone, et la touche par une dimension au moins :
    /// ni débordement, ni image inutilement petite.
    #[test]
    fn l_image_tient_dans_la_zone_et_la_touche() {
        for image in [(997, 692), (338, 614), (511, 902), (220, 230)] {
            let (l, h) = inscrire(image, ZONE);
            assert!(
                l <= ZONE.0 + 1e-3 && h <= ZONE.1 + 1e-3,
                "{image:?} déborde"
            );
            let touche = (l - ZONE.0).abs() < 1e-3 || (h - ZONE.1).abs() < 1e-3;
            assert!(touche, "{image:?} → {l}×{h} ne touche aucun bord");
        }
    }

    /// Le cas `LDK2`, qui est le défaut à corriger : une couverture paysage
    /// laisse du vide en haut et en bas, et c'est normal — ce qui ne l'est pas,
    /// c'est que la tuile rétrécisse d'autant. La zone étant réservée en
    /// entier, la hauteur inscrite est **inférieure** à la zone, et la tuile
    /// n'en sait rien.
    #[test]
    fn une_couverture_paysage_laisse_du_vide_sans_reduire_la_zone() {
        let (l, h) = inscrire((997, 692), ZONE);
        assert!((l - LARGEUR_TUILE).abs() < 1e-3, "la largeur contraint");
        assert!(
            h < HAUTEUR_COUVERTURE - 50.0,
            "il reste du vide vertical : {h} sur {HAUTEUR_COUVERTURE}"
        );
    }

    /// Une couverture portrait, elle, est contrainte par la hauteur.
    #[test]
    fn une_couverture_portrait_est_contrainte_par_la_hauteur() {
        let (l, h) = inscrire((511, 902), ZONE);
        assert!((h - HAUTEUR_COUVERTURE).abs() < 1e-3);
        assert!(l < LARGEUR_TUILE);
    }

    /// Une image plus petite que la zone est **agrandie**, sans quoi elle
    /// laisserait un trou dans la grille.
    ///
    /// # Ce test a d'abord menti
    ///
    /// Il employait `RA05`, 338 × 614, en la disant « petite ». Elle est plus
    /// grande que la zone (220 × 230) dans les **deux** dimensions : elle est
    /// donc réduite, et le test passait avec ou sans l'agrandissement — une
    /// mutation plafonnant l'échelle à 1 ne le faisait pas broncher. Il faut
    /// une image réellement plus petite, et aucune des neuf couvertures
    /// réelles ne l'est : d'où la vignette inventée.
    #[test]
    fn une_petite_image_est_agrandie() {
        let (l, h) = inscrire((110, 115), ZONE);
        assert!(
            (l - LARGEUR_TUILE).abs() < 1e-3 && (h - HAUTEUR_COUVERTURE).abs() < 1e-3,
            "la vignette est doublée pour remplir la zone : {l}×{h}"
        );

        // Et les vraies couvertures, elles, sont bien réduites.
        let (l, h) = inscrire((338, 614), ZONE);
        assert!(l < 338.0 && h < 614.0, "RA05 est réduite, pas agrandie");
        assert!(
            (h - HAUTEUR_COUVERTURE).abs() < 1e-3,
            "elle remplit la hauteur"
        );
    }

    /// Ce que le décodeur n'a pas su mesurer ne produit pas un rectangle
    /// infini.
    #[test]
    fn une_taille_nulle_ne_produit_pas_de_rectangle() {
        assert_eq!(inscrire((0, 100), ZONE), (0.0, 0.0));
        assert_eq!(inscrire((100, 0), ZONE), (0.0, 0.0));
        assert_eq!(inscrire((0, 0), ZONE), (0.0, 0.0));
        assert_eq!(inscrire((100, 100), (0.0, 230.0)), (0.0, 0.0));
        assert_eq!(inscrire((100, 100), (220.0, -1.0)), (0.0, 0.0));
    }
}
