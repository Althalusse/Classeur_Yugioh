// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! L'écran classeur — la double page.
//!
//! Portage de la part visuelle d'`ui/ecran_classeur.py`. Tout ce qui se
//! calcule vit dans [`ygo_app::classeur`] et y est éprouvé contre le Python ;
//! il ne reste ici que le dessin.
//!
//! # Le premier écart DÉLIBÉRÉ avec le Python
//!
//! Jusqu'ici la règle était la fidélité, bugs latents compris (règle R9). Ici
//! non : l'auteur du projet n'était pas satisfait de l'ergonomie de sa V1, et
//! demande explicitement de réduire le nombre de clics. La **donnée** reste
//! identique — `ygo_app::possession` porte le même `UPDATE` —, seule la
//! **commande** change.
//!
//! | Python V1 | ici |
//! |---|---|
//! | un bouton ✓ en haut à gauche, qui bascule 0 ↔ 1 | **`−` et `+`** aux deux coins hauts, quantité au centre |
//! | rien | **molette** sur la carte : ±1 sans un clic |
//! | rien | **maintien** du bouton : répétition |
//! | rien | **← →** pour feuilleter, `Ctrl+F`, `Origine`/`Fin`, `Échap` |
//! | clic droit : menu contextuel | conservé, avec « remettre à zéro » |
//!
//! Les badges **OVERFRAME** et **PLAYSET** gardent leur place d'origine, en bas
//! à droite et en bas à gauche : ils sont bons et l'utilisateur les reconnaît.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui;
use ygo_app::classeur::{self, BornesCarte, Carte, Filtres, Grille, Possession};
use ygo_core::config::{Config, SourceImage, GRILLE_MAX, GRILLE_MIN};
use ygo_core::paths::Paths;
use ygo_core::rarity::Priorites;

use crate::attenuation::Facteurs;
use crate::images::{Cache, Cle};

/// Bornes de largeur d'une carte, en points.
///
/// Le minimum vient du Python (`CARD_W_MIN`). Le maximum, lui, **n'est plus
/// celui du Python** : `CARD_W_MAX` valait 220, une valeur qui payait le
/// redimensionnement Pillow refait à chaque taille. egui garde une texture
/// unique et laisse le GPU l'échelonner : agrandir une carte ne coûte plus
/// rien, et la borne n'avait plus qu'un effet, celui de laisser 1 100 points
/// de noir sur un écran 1440p.
///
/// Ce qui borne pour de bon, c'est la définition des images. Mesurée sur
/// l'installation réelle : **1 872 des 1 878 fichiers du cache font
/// 813 × 1 185 pixels** (six exemplaires plus anciens tournent autour de
/// 410). Au-delà de 813 points, on agrandirait au-delà de la définition et
/// l'image deviendrait floue.
///
/// En pratique cette borne ne sert jamais : sur 2 560 × 1 440, l'ajustement
/// en hauteur donne 300 points. Elle est un garde-fou, pas une politique.
pub const BORNES: BornesCarte = BornesCarte { min: 90, max: 813 };

/// L'écran, avec tout ce qu'il lui faut pour se dessiner.
pub struct EcranClasseur {
    paths: Paths,
    code: String,
    priorites: Priorites,
    source: SourceImage,
    grille: Grille,

    /// Toutes les cartes, telles que chargées.
    toutes: Vec<Carte>,
    /// Celles que les filtres laissent passer.
    visibles: Vec<Carte>,
    raretes: Vec<String>,

    filtres: Filtres,
    francais: bool,
    double_page: usize,

    cache: Cache,
    survolee: Option<i64>,
    /// La fiche ouverte au clic, si elle l'est.
    fiche: crate::fiche::EtatFiche,
    /// Connexion en ÉCRITURE — l'écran est le seul écrivain de ce classeur
    /// tant qu'il est ouvert (règle R3, un écrivain par base).
    conn: ygo_db::rusqlite::Connection,
    /// Ce qu'un clic a demandé, appliqué après le dessin pour ne pas muter
    /// la liste qu'on est en train de parcourir.
    en_attente: Vec<Action>,
    /// `Ctrl+F` demande le focus sur la recherche à la prochaine image.
    focus_recherche: bool,
    /// Un retour à l'accueil a été demandé — bouton ou `Alt+←`.
    retour: bool,
    /// Un passage aux options a été demandé.
    options: bool,
    /// Le `set_code` de la carte dont on veut voir les artworks.
    artworks: Option<String>,
    /// La bascule de masse en cours de confirmation, et l'état lu **une
    /// fois** à l'ouverture de la boîte.
    a_basculer: Option<(ygo_app::possession::Bascule, ygo_app::possession::Etat)>,
    /// Les réglages propres au classeur, en cours d'édition.
    ///
    /// `None` quand le panneau est fermé. Ouvert, il porte une **copie** :
    /// rien n'est écrit tant que l'utilisateur n'a pas validé, et « Annuler »
    /// n'a alors rien à défaire.
    reglages: Option<ygo_app::classeur::Reglages>,
    /// Les propositions de la recherche, recalculées à chaque frappe.
    propositions: Vec<classeur::Suggestion>,
    /// La carte vers laquelle une proposition a envoyé, et depuis quand.
    ///
    /// Elle est encadrée d'or le temps qu'on la retrouve des yeux. Sans ce
    /// repère, être « envoyé à la page » laisse l'utilisateur devant neuf
    /// cartes sans lui dire laquelle il cherchait.
    surlignee: Option<(i64, f64)>,
    /// Dernier pas de répétition émis par chaque bouton maintenu.
    dernier_pas: std::collections::HashMap<egui::Id, f64>,
    /// La page que le champ de saut vise.
    page_visee: usize,
    /// Largeur du groupe de navigation, mesurée à l'image précédente.
    largeur_navigation: f32,

    /// Ce que la création a corrigé ou refusé sur ce classeur, s'il y a
    /// quelque chose à en dire.
    ///
    /// Lu **une fois** à l'ouverture : c'est un état figé au moment de la
    /// création, que rien dans cet écran ne fait bouger.
    ecarts: Option<ygo_app::classeur::Ecarts>,
    /// Le détail des écarts est-il déplié ?
    detail_ecarts: bool,

    /// Mesures, affichées en bas — c'est l'épreuve que l'arbitrage n'a pas faite.
    duree_chargement: Duration,
    duree_filtre: Duration,
    /// Temps de lecture de la dernière fiche ouverte.
    duree_fiche: Duration,
    depart: Instant,
    premier_rendu: Option<Duration>,
}

impl std::fmt::Debug for EcranClasseur {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EcranClasseur")
            .field("code", &self.code)
            .field("cartes", &self.toutes.len())
            .field("visibles", &self.visibles.len())
            .field("double_page", &self.double_page)
            .finish()
    }
}

impl EcranClasseur {
    /// Ouvre un classeur et charge ses cartes.
    ///
    /// # Errors
    ///
    /// Si la base du classeur est absente ou illisible.
    pub fn ouvrir(racine: PathBuf, code: &str) -> anyhow::Result<Self> {
        let paths = Paths::depuis_racine(racine);
        let code = code.trim().to_uppercase();
        let chemin = paths.classeur_db(&code);

        let config = Config::charger(paths.app_config());
        let priorites = Priorites::charger(paths.rarity_config());
        let grille = classeur::grille_du_classeur(&chemin, config.grille_defaut());
        let francais = config.langue().code() == "FR";
        let source = config.source_image();

        let depart = Instant::now();
        let conn = ygo_db::connexion::ouvrir(&chemin)?;
        // L'ordre du classeur s'il en a un, celui des Options sinon — même
        // règle que la grille, juste au-dessus.
        let ordre = classeur::ordre_du_classeur(&chemin, config.ordre_tri());
        let toutes = classeur::charger(&conn, francais, source, ordre, &priorites)?;
        let duree_chargement = depart.elapsed();

        let ecarts = ygo_app::classeur::ecarts_du_classeur(&chemin);
        let raretes = classeur::raretes_disponibles(&toutes);
        let filtres = Filtres::default();
        let debut = Instant::now();
        let visibles = classeur::appliquer(toutes.clone(), &filtres, &priorites);
        let duree_filtre = debut.elapsed();

        println!(
            "{code} — {} cartes lues en {duree_chargement:?}, grille {}×{}, {} double(s) page(s)",
            toutes.len(),
            grille.colonnes,
            grille.lignes,
            classeur::nb_doubles_pages(visibles.len(), grille)
        );

        Ok(Self {
            paths,
            code,
            priorites,
            source,
            grille,
            toutes,
            visibles,
            raretes,
            filtres,
            francais,
            double_page: 0,
            cache: Cache::default(),
            fiche: crate::fiche::EtatFiche::default(),
            survolee: None,
            conn,
            en_attente: Vec::new(),
            artworks: None,
            a_basculer: None,
            reglages: None,
            focus_recherche: false,
            retour: false,
            options: false,
            propositions: Vec::new(),
            surlignee: None,
            dernier_pas: std::collections::HashMap::new(),
            duree_chargement,
            duree_filtre,
            duree_fiche: Duration::ZERO,
            page_visee: 1,
            largeur_navigation: 0.0,
            ecarts,
            detail_ecarts: false,
            depart: Instant::now(),
            premier_rendu: None,
        })
    }

    /// Le code du set ouvert.
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Reprend la demande d'ouverture des options.
    ///
    /// Les options sont accessibles depuis le classeur autant que depuis
    /// l'accueil : c'est en regardant un classeur qu'on se rend compte qu'un
    /// réglage ne convient pas. L'application retient d'où l'on vient et y
    /// revient.
    pub fn options_demandees(&mut self) -> bool {
        std::mem::take(&mut self.options)
    }

    /// Reprend la carte dont on veut voir les artworks, s'il y en a une.
    pub fn artworks_demandes(&mut self) -> Option<String> {
        std::mem::take(&mut self.artworks)
    }

    /// Reprend la demande de retour à l'accueil, s'il y en a une.
    ///
    /// L'appelant la consomme. C'est lui qui décide ce que « revenir » veut
    /// dire ; l'écran se contente de dire que l'utilisateur l'a demandé.
    pub fn retour_demande(&mut self) -> bool {
        std::mem::take(&mut self.retour)
    }

    /// Recalcule les cartes visibles après un changement de filtre.
    /// Recalcule les propositions de la recherche.
    ///
    /// Elles portent sur **toutes** les cartes, pas sur les visibles : leur
    /// rôle est justement d'emmener quelque part dans le classeur entier.
    fn reproposer(&mut self) {
        self.propositions = classeur::suggestions(
            &self.toutes,
            &self.filtres.terme,
            self.grille,
            classeur::MAX_SUGGESTIONS,
        );
    }

    /// Va au feuillet d'une proposition et marque la carte.
    ///
    /// La recherche est **effacée** au passage : on va voir la carte dans son
    /// classeur, à sa place, entourée de ses voisines. Isoler et se déplacer
    /// sont deux gestes différents — la touche Entrée fait l'autre.
    fn aller_a(&mut self, suggestion: &classeur::Suggestion, maintenant: f64) {
        self.filtres.terme.clear();
        self.propositions.clear();
        self.refiltrer();
        let position = self
            .visibles
            .iter()
            .position(|c| c.rowid == suggestion.rowid)
            .unwrap_or(suggestion.position);
        self.double_page = classeur::feuillet_de(position, self.grille);
        self.surlignee = Some((suggestion.rowid, maintenant));
    }

    fn refiltrer(&mut self) {
        let debut = Instant::now();
        self.visibles = classeur::appliquer(self.toutes.clone(), &self.filtres, &self.priorites);
        self.duree_filtre = debut.elapsed();
        // Un filtre qui réduit le classeur peut laisser la page courante
        // au-delà de la fin : on ramène dans les bornes plutôt que d'afficher
        // du vide sans explication.
        let dernier = classeur::nb_doubles_pages(self.visibles.len(), self.grille) - 1;
        self.double_page = self.double_page.min(dernier);
    }

    /// Oublie les fichiers réputés absents.
    ///
    /// Appelé par l'application quand une passe de téléchargement vient
    /// d'écrire de nouveaux fichiers : les cartes qui montraient un
    /// emplacement vide retentent leur lecture, les textures déjà décodées
    /// restent en place.
    pub fn rafraichir_images(&mut self) {
        self.cache.oublier_manquants();
    }

    /// Le dossier où sont les images de cartes.
    fn dossier_images(&self) -> PathBuf {
        self.paths.img_small()
    }
}

impl eframe::App for EcranClasseur {
    fn ui(&mut self, racine: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.premier_rendu.is_none() {
            let ecoule = self.depart.elapsed();
            self.premier_rendu = Some(ecoule);
            println!("       premier rendu à {ecoule:?}");
        }

        self.raccourcis(racine.ctx());
        self.barre_du_haut(racine);
        self.bandeau_ecarts(racine);
        self.barre_du_bas(racine);
        self.pages(racine);
        self.montrer_la_fiche(racine.ctx());
        self.confirmation_bascule(racine.ctx());
        self.panneau_reglages(racine.ctx());
        self.appliquer_actions();
    }
}

impl EcranClasseur {
    /// Ouvre la fiche sur une ligne du classeur.
    ///
    /// Une lecture ratée n'ouvre rien : mieux vaut ne rien voir se passer
    /// qu'une fenêtre vide dont on ne saurait quoi penser. La trace, elle,
    /// est écrite.
    fn ouvrir_la_fiche(&mut self, rowid: i64) {
        let depart = Instant::now();
        match ygo_app::fiche::lire(&self.paths, &self.code, rowid, self.source) {
            Ok(Some(f)) => {
                // Mesurée et affichée comme le reste : c'est cette lecture
                // qui, construite en table de hachage, gelait l'application
                // une demi-seconde à chaque clic.
                self.duree_fiche = depart.elapsed();
                self.fiche.ouvrir(f);
            }
            Ok(None) => tracing::warn!(rowid, "fiche : ligne introuvable"),
            Err(e) => tracing::warn!(rowid, erreur = %e, "fiche : lecture impossible"),
        }
    }

    /// Dessine la fiche par-dessus le classeur, et obéit à ce qu'elle demande.
    ///
    /// Les flèches déplacent la fiche **dans la page affichée**, pas dans tout
    /// le classeur : on regarde le feuillet qu'on a sous les yeux, et sauter à
    /// une carte d'une autre page sans que la page bouge serait incompréhensible.
    fn montrer_la_fiche(&mut self, ctx: &egui::Context) {
        if !self.fiche.est_ouverte() {
            return;
        }
        let dossier = self.dossier_images();
        let demande = self
            .fiche
            .afficher(ctx, &mut self.cache, &dossier, self.francais);
        match demande {
            crate::fiche::Demande::Rien => {}
            crate::fiche::Demande::Fermer => self.fiche.fermer(),
            crate::fiche::Demande::Precedente => self.deplacer_la_fiche(-1),
            crate::fiche::Demande::Suivante => self.deplacer_la_fiche(1),
        }
    }

    /// Passe à la carte voisine de la double page affichée.
    fn deplacer_la_fiche(&mut self, pas: isize) {
        let Some(actuelle) = self.fiche.carte().map(|f| f.rowid) else {
            return;
        };
        let (gauche, droite) = classeur::double_page(&self.visibles, self.double_page, self.grille);
        let rangs: Vec<i64> = gauche
            .iter()
            .chain(droite.iter())
            .map(|c| c.rowid)
            .collect();
        let Some(i) = rangs.iter().position(|r| *r == actuelle) else {
            return;
        };
        let vise = crate::fiche::voisine(i, rangs.len(), pas);
        if vise != i {
            if let Some(rowid) = rangs.get(vise).copied() {
                self.ouvrir_la_fiche(rowid);
            }
        }
    }

    /// Titre, compteurs et filtres.
    fn barre_du_haut(&mut self, racine: &mut egui::Ui) {
        egui::Panel::top("entete").show(racine, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui
                    .button("◀ Mes classeurs")
                    .on_hover_text("Retour à l'accueil (Alt+←)")
                    .clicked()
                {
                    self.retour = true;
                }
                ui.add_space(8.0);
                ui.heading(&self.code);
                ui.add_space(12.0);
                let possedees = self.visibles.iter().filter(|c| c.quantite > 0).count();
                ui.label(format!(
                    "{} carte(s) affichée(s) sur {} · {possedees} possédée(s)",
                    self.visibles.len(),
                    self.toutes.len()
                ));

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("⚙").on_hover_text("Options").clicked() {
                        self.options = true;
                    }
                    ui.add_space(8.0);
                    self.menu_du_classeur(ui);
                    ui.add_space(8.0);
                    if ui.checkbox(&mut self.francais, "FR").changed() {
                        self.recharger();
                    }
                    ui.add_space(8.0);

                    let mut possession = self.filtres.possession;
                    egui::ComboBox::from_id_salt("possession")
                        .selected_text(match possession {
                            Possession::Toutes => "Toutes",
                            Possession::Possedees => "Possédées",
                            Possession::NonPossedees => "Non possédées",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut possession, Possession::Toutes, "Toutes");
                            ui.selectable_value(
                                &mut possession,
                                Possession::Possedees,
                                "Possédées",
                            );
                            ui.selectable_value(
                                &mut possession,
                                Possession::NonPossedees,
                                "Non possédées",
                            );
                        });
                    if possession != self.filtres.possession {
                        self.filtres.possession = possession;
                        self.refiltrer();
                    }

                    let mut rarete = self.filtres.rarete.clone();
                    egui::ComboBox::from_id_salt("rarete")
                        .selected_text(rarete.as_deref().unwrap_or("Toutes les raretés"))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut rarete, None, "Toutes les raretés");
                            for r in &self.raretes {
                                ui.selectable_value(&mut rarete, Some(r.clone()), r);
                            }
                        });
                    if rarete != self.filtres.rarete {
                        self.filtres.rarete = rarete;
                        self.refiltrer();
                    }

                    let recherche = ui.add(
                        egui::TextEdit::singleline(&mut self.filtres.terme)
                            .hint_text("nom ou numéro…  (Ctrl+F)")
                            .desired_width(200.0),
                    );
                    if std::mem::take(&mut self.focus_recherche) {
                        recherche.request_focus();
                    }
                    if recherche.changed() {
                        self.reproposer();
                    }
                    // Entrée isole la recherche dans le classeur, comme avant.
                    if recherche.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        self.propositions.clear();
                        self.refiltrer();
                    }
                    self.liste_propositions(ui, &recherche);
                });
            });
            ui.add_space(8.0);
        });
    }

    /// La ligne qui dit ce que la création a corrigé ou refusé.
    ///
    /// # Pourquoi une ligne, et pas un journal ailleurs
    ///
    /// Un classeur annoncé à 68 cartes qui en montre 62 pose une question, et
    /// la question se pose **devant le classeur**. Un fichier de log, un
    /// écran d'anomalies, un rapport dans les Options : tous demandent de
    /// savoir qu'il faut aller voir. Cette ligne, non — elle est là quand il
    /// y a quelque chose, et absente le reste du temps, ce qui est le cas
    /// courant.
    ///
    /// Elle est discrète par construction : une ligne, repliée, sur le fond
    /// de la fenêtre. Le détail se déplie au clic et ne s'impose jamais.
    fn bandeau_ecarts(&mut self, racine: &mut egui::Ui) {
        let Some(ecarts) = self.ecarts.clone() else {
            return;
        };
        let Some(resume) = ecarts.resume() else {
            return;
        };
        egui::Panel::top("ecarts").show(racine, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("⚠").color(egui::Color32::from_rgb(210, 160, 60)));
                ui.label(egui::RichText::new(resume).weak());
                let bouton = if self.detail_ecarts {
                    "masquer"
                } else {
                    "voir"
                };
                if ui
                    .small_button(bouton)
                    .on_hover_text("Ce qui a été corrigé ou écarté à la création")
                    .clicked()
                {
                    self.detail_ecarts = !self.detail_ecarts;
                }
            });
            if self.detail_ecarts {
                ui.add_space(4.0);
                ui.indent("detail_ecarts", |ui| {
                    if !ecarts.quand.is_empty() {
                        ui.label(
                            egui::RichText::new(format!("Classeur créé le {}", ecarts.quand))
                                .weak()
                                .small(),
                        );
                    }
                    for fantome in &ecarts.fantomes {
                        ui.label(format!(
                            "• {} — « {} » écartée : cette mention n'est pas une rareté, \
                             la même illustration porte déjà {}",
                            fantome.set_code,
                            fantome.rarete,
                            fantome.jumelles.join(", ")
                        ));
                    }
                    for (libelle, n) in &ecarts.inconnues {
                        ui.label(format!(
                            "• « {libelle} » — {n} ligne(s) gardée(s) telles quelles : \
                             libellé inconnu du référentiel, rien n'a été supposé"
                        ));
                    }
                    if ecarts.canonisees > 0 {
                        ui.label(format!(
                            "• {} ligne(s) dont le libellé a été ramené à la forme des Options \
                             (« UR » → « Ultra Rare »), pour qu'elles se rangent au bon endroit",
                            ecarts.canonisees
                        ));
                    }
                });
            }
            ui.add_space(4.0);
        });
    }

    /// La liste des correspondances, sous le champ de recherche.
    ///
    /// Un clic **emmène** à la page où la carte se trouve, dans le classeur
    /// entier. La touche Entrée, elle, **isole** — c'est le comportement
    /// d'avant, et les deux ont leur usage : on cherche tantôt à voir une
    /// carte à sa place, tantôt à ne voir qu'elle.
    fn liste_propositions(&mut self, ui: &mut egui::Ui, ancre: &egui::Response) {
        if self.propositions.is_empty() {
            return;
        }
        let mut choisie: Option<classeur::Suggestion> = None;
        egui::Popup::from_response(ancre)
            .id(egui::Id::new("propositions"))
            .open(true)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.set_min_width(360.0);
                for proposition in &self.propositions {
                    let ligne = ui.add(
                        egui::Button::new(format!(
                            "{}   ·   {}   ·   {}   →  feuillet {}",
                            proposition.set_code,
                            proposition.nom,
                            proposition.rarete,
                            proposition.feuillet + 1
                        ))
                        .frame(false),
                    );
                    if ligne.clicked() {
                        choisie = Some(proposition.clone());
                    }
                }
                ui.separator();
                ui.small("Entrée : n'afficher que les résultats");
            });
        if let Some(proposition) = choisie {
            let maintenant = ui.input(|i| i.time);
            self.aller_a(&proposition, maintenant);
            ui.ctx().memory_mut(|m| {
                if let Some(id) = m.focused() {
                    m.surrender_focus(id);
                }
            });
        }
    }

    /// Les raccourcis clavier, aux conventions de Windows.
    ///
    /// Aucun n'agit quand une zone de saisie a le focus : taper « f » dans la
    /// recherche ne doit pas feuilleter le classeur.
    fn raccourcis(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            // Une exception : Échap rend la main même depuis la recherche.
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.filtres.terme.clear();
                self.propositions.clear();
                self.refiltrer();
                ctx.memory_mut(|m| {
                    if let Some(id) = m.focused() {
                        m.surrender_focus(id);
                    }
                });
            }
            return;
        }
        let total = classeur::nb_doubles_pages(self.visibles.len(), self.grille);
        let (precedent, suivant, debut, fin, cherche, retour) = ctx.input(|i| {
            // `Alt+←` est le « précédent » de Windows. Il est lu AVANT la
            // flèche nue, et l'exclut : sans quoi le même appui feuilletterait
            // le classeur en même temps qu'il le quitterait.
            let arriere = i.modifiers.alt && i.key_pressed(egui::Key::ArrowLeft);
            (
                !arriere
                    && (i.key_pressed(egui::Key::ArrowLeft) || i.key_pressed(egui::Key::PageUp)),
                i.key_pressed(egui::Key::ArrowRight) || i.key_pressed(egui::Key::PageDown),
                i.key_pressed(egui::Key::Home),
                i.key_pressed(egui::Key::End),
                i.modifiers.command && i.key_pressed(egui::Key::F),
                arriere,
            )
        });
        if retour {
            self.retour = true;
        }
        if precedent {
            self.double_page = self.double_page.saturating_sub(1);
        }
        if suivant {
            self.double_page = (self.double_page + 1).min(total.saturating_sub(1));
        }
        if debut {
            self.double_page = 0;
        }
        if fin {
            self.double_page = total.saturating_sub(1);
        }
        if cherche {
            self.focus_recherche = true;
        }
    }

    /// Le menu des actions qui portent sur le classeur entier.
    ///
    /// Un menu et non deux boutons : la barre du haut porte déjà le retour,
    /// le titre, les compteurs, trois filtres, la case FR et les options.
    /// Deux boutons de plus la rendraient illisible pour des actions qu'on
    /// déclenche une fois par classeur.
    ///
    /// Les entrées sont **montrées mais désactivées** quand elles n'ont rien
    /// à faire, avec la raison en infobulle — comme le menu contextuel des
    /// cartes. Les cacher donnerait l'illusion d'un menu complet.
    fn menu_du_classeur(&mut self, ui: &mut egui::Ui) {
        use ygo_app::possession::Bascule;
        // L'état se lit une fois par ouverture du menu, pas à chaque image.
        let etat = ygo_app::possession::etat(&self.conn).unwrap_or_default();
        ui.menu_button("⋯", |ui| {
            ui.label(egui::RichText::new("Tout le classeur").strong());
            ui.separator();
            for (bascule, actif, raison) in [
                (
                    Bascule::ToutPosseder,
                    etat.incomplet(),
                    "Tout est déjà possédé",
                ),
                (
                    Bascule::ToutRemettreAZero,
                    etat.a_quelque_chose(),
                    "Rien n'est possédé",
                ),
            ] {
                let bouton =
                    ui.add_enabled(actif, egui::Button::new(format!("{}…", bascule.libelle())));
                if bouton.clicked() {
                    self.a_basculer = Some((bascule, etat));
                    ui.close();
                }
                if !actif {
                    bouton.on_disabled_hover_text(raison);
                }
            }
            ui.separator();
            if ui
                .button("Réglages du classeur…")
                .on_hover_text("Grille et ordre de tri, pour ce classeur seulement")
                .clicked()
            {
                self.reglages = Some(ygo_app::classeur::reglages_du_classeur(
                    &self.paths.classeur_db(&self.code),
                ));
                ui.close();
            }
        })
        .response
        .on_hover_text("Actions sur tout le classeur");
    }

    /// La confirmation d'une bascule de masse.
    ///
    /// # Pourquoi « Tout possédé » se confirme aussi
    ///
    /// Cocher un classeur entier ne détruit rien en soi — mais l'annuler
    /// demande de tout décocher, ce qui emporte alors les quantités
    /// réellement saisies. Les deux sens passent donc par la même porte.
    fn confirmation_bascule(&mut self, ctx: &egui::Context) {
        use ygo_app::possession::Bascule;
        let Some((bascule, etat)) = self.a_basculer else {
            return;
        };
        let mut ouverte = true;
        let mut confirme = false;
        egui::Window::new(format!("{} — {} ?", bascule.libelle(), self.code))
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
            self.en_attente.push(Action::Basculer(bascule));
            self.a_basculer = None;
        } else if !ouverte {
            self.a_basculer = None;
        }
    }

    /// Les réglages propres au classeur : grille et ordre de tri.
    ///
    /// # Pourquoi « suivre les Options » est une case et non une valeur
    ///
    /// Décocher la case **efface** la clé en base plutôt que d'y recopier le
    /// défaut du moment. La nuance se voit demain : un classeur qui aurait
    /// figé « 3×3 » resterait à 3×3 après un changement d'Options, sans que
    /// rien ne le dise. Coché, il suit ; décoché, il décide.
    ///
    /// # Ce que valider coûte
    ///
    /// Un rechargement complet du classeur — la grille change la pagination,
    /// l'ordre change la suite des cartes. C'est six millisecondes de lecture
    /// sur les 1 120 lignes de `RA02`, et c'est plus sûr que de recalculer
    /// deux états en place.
    fn panneau_reglages(&mut self, ctx: &egui::Context) {
        use ygo_app::classeur::Grille;
        let Some(mut reglages) = self.reglages else {
            return;
        };
        let defaut = {
            let config = Config::charger(self.paths.app_config());
            (config.grille_defaut(), config.ordre_tri())
        };
        let mut ouverte = true;
        let mut valide = false;
        egui::Window::new(format!("Réglages — {}", self.code))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .open(&mut ouverte)
            .show(ctx, |ui| {
                ui.set_min_width(480.0);
                ui.add_space(4.0);
                ui.small(
                    "Ces réglages ne valent que pour ce classeur. Décochez pour \
                     revenir à ce que disent les Options.",
                );
                ui.add_space(10.0);

                // ── La grille ────────────────────────────────────────────
                let mut suit_grille = reglages.grille.is_none();
                if ui
                    .checkbox(
                        &mut suit_grille,
                        format!(
                            "Grille : suivre les Options ({}×{})",
                            defaut.0 .0, defaut.0 .1
                        ),
                    )
                    .changed()
                {
                    reglages.grille = if suit_grille {
                        None
                    } else {
                        Some(Grille {
                            colonnes: defaut.0 .0,
                            lignes: defaut.0 .1,
                        })
                    };
                }
                if let Some(grille) = &mut reglages.grille {
                    ui.horizontal(|ui| {
                        ui.add_space(24.0);
                        ui.label("Colonnes");
                        ui.add(
                            egui::DragValue::new(&mut grille.colonnes)
                                .range(GRILLE_MIN..=GRILLE_MAX)
                                .speed(0.2),
                        );
                        ui.add_space(12.0);
                        ui.label("Lignes");
                        ui.add(
                            egui::DragValue::new(&mut grille.lignes)
                                .range(GRILLE_MIN..=GRILLE_MAX)
                                .speed(0.2),
                        );
                        ui.add_space(12.0);
                        ui.small(
                            egui::RichText::new(format!(
                                "{} carte(s) par page",
                                u32::from(grille.colonnes) * u32::from(grille.lignes)
                            ))
                            .weak(),
                        );
                    });
                }

                ui.add_space(12.0);

                // ── L'ordre de tri ───────────────────────────────────────
                let mut suit_ordre = reglages.ordre.is_none();
                if ui
                    .checkbox(&mut suit_ordre, "Ordre de tri : suivre les Options")
                    .changed()
                {
                    reglages.ordre = if suit_ordre { None } else { Some(defaut.1) };
                }
                if let Some(ordre) = reglages.ordre {
                    ui.add_space(4.0);
                    let mut nouvel_ordre = None;
                    ui.horizontal(|ui| {
                        ui.add_space(24.0);
                        ui.vertical(|ui| {
                            for (rang, critere) in ordre.into_iter().enumerate() {
                                let id = egui::Id::new(("tri-classeur", critere.code()));
                                let (_, depose) = ui.dnd_drop_zone::<usize, ()>(
                                    egui::Frame::new()
                                        .inner_margin(egui::Margin::symmetric(6, 4))
                                        .fill(ui.visuals().faint_bg_color)
                                        .corner_radius(4),
                                    |ui| {
                                        ui.dnd_drag_source(id, rang, |ui| {
                                            ui.horizontal(|ui| {
                                                crate::options::poignee(ui);
                                                ui.add_space(8.0);
                                                ui.label(
                                                    egui::RichText::new(format!("{}.", rang + 1))
                                                        .weak(),
                                                );
                                                ui.add_space(4.0);
                                                ui.add(
                                                    egui::Label::new(
                                                        crate::options::libelle_critere(critere),
                                                    )
                                                    .selectable(false),
                                                );
                                            });
                                        });
                                    },
                                );
                                if let Some(depuis) = depose {
                                    nouvel_ordre =
                                        Some(crate::options::deplacer(ordre, *depuis, rang));
                                }
                                ui.add_space(2.0);
                            }
                            ui.small("Attrapez une ligne pour la déplacer.");
                        });
                    });
                    if let Some(neuf) = nouvel_ordre {
                        reglages.ordre = Some(neuf);
                    }
                }

                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    if ui.button("Annuler").clicked() {
                        self.reglages = None;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Appliquer").clicked() {
                            valide = true;
                        }
                    });
                });
                ui.add_space(4.0);
            });

        if valide {
            let chemin = self.paths.classeur_db(&self.code);
            // Par NOTRE connexion : l'écran est le seul écrivain de ce
            // classeur tant qu'il est ouvert (règle R3). En ouvrir une
            // seconde pour trois lignes de `meta` serait exactement ce que la
            // règle interdit.
            match ygo_app::classeur::definir_reglages_sur(&mut self.conn, &reglages) {
                Ok(()) => {
                    self.grille = ygo_app::classeur::grille_du_classeur(&chemin, defaut.0);
                    // La page courante peut ne plus exister : une grille plus
                    // grande tient en moins de feuillets.
                    self.double_page = 0;
                    self.recharger();
                }
                Err(e) => {
                    tracing::warn!(classeur = %self.code, erreur = %e, "réglages non écrits");
                }
            }
            self.reglages = None;
        } else if !ouverte {
            self.reglages = None;
        } else if self.reglages.is_some() {
            // Le panneau garde ce que l'utilisateur vient de changer tant
            // qu'il ne l'a ni validé ni fermé.
            self.reglages = Some(reglages);
        }
    }

    /// Applique ce que les commandes ont demandé, une fois le dessin fini.
    fn appliquer_actions(&mut self) {
        if self.en_attente.is_empty() {
            return;
        }
        let mut touchees = Vec::new();
        for action in std::mem::take(&mut self.en_attente) {
            let (rowid, resultat) = match action {
                Action::Ajuster { rowid, delta } => (
                    rowid,
                    ygo_app::possession::ajuster(&mut self.conn, rowid, delta),
                ),
                Action::RemettreAZero { rowid } => (
                    rowid,
                    ygo_app::possession::regler_quantite(&self.conn, rowid, 0).map(|_| Some(0)),
                ),
                Action::OuvrirFiche(rowid) => {
                    self.ouvrir_la_fiche(rowid);
                    continue;
                }
                // L'écran est le seul écrivain de ce classeur tant qu'il est
                // ouvert (règle R3) : la bascule passe par SA connexion, pas
                // par `appliquer_au_classeur`, qui en ouvrirait une seconde.
                Action::Basculer(bascule) => {
                    let resultat = match bascule {
                        ygo_app::possession::Bascule::ToutPosseder => {
                            ygo_app::possession::tout_posseder(&self.conn)
                        }
                        ygo_app::possession::Bascule::ToutRemettreAZero => {
                            ygo_app::possession::tout_remettre_a_zero(&self.conn)
                        }
                    };
                    match resultat {
                        Ok(n) => {
                            tracing::info!(classeur = %self.code, lignes = n, "bascule de masse");
                            self.recharger();
                        }
                        Err(e) => {
                            tracing::warn!(classeur = %self.code, erreur = %e, "bascule refusée");
                        }
                    }
                    continue;
                }
            };
            match resultat {
                Ok(Some(quantite)) => touchees.push((rowid, quantite)),
                Ok(None) => {}
                Err(e) => tracing::warn!(rowid, erreur = %e, "quantité non modifiée"),
            }
        }

        // On met la mémoire à jour plutôt que de tout relire : recharger 1 120
        // cartes et les retrier à chaque clic serait absurde.
        for (rowid, quantite) in touchees {
            for liste in [&mut self.toutes, &mut self.visibles] {
                if let Some(carte) = liste.iter_mut().find(|c| c.rowid == rowid) {
                    carte.quantite = quantite;
                    carte.possedee = quantite > 0;
                }
            }
        }
        // Le filtre de possession peut faire sortir la carte de la page :
        // c'est voulu, et c'est ce que fait le Python.
        if self.filtres.possession != Possession::Toutes || self.filtres.n_raretes > 0 {
            self.refiltrer();
        }
    }

    /// Recharge les cartes — après un changement de langue.
    fn recharger(&mut self) {
        let Ok(conn) = ygo_db::connexion::ouvrir_lecture_seule(self.paths.classeur_db(&self.code))
        else {
            return;
        };
        let config = Config::charger(self.paths.app_config());
        let chemin = self.paths.classeur_db(&self.code);
        if let Ok(cartes) = classeur::charger(
            &conn,
            self.francais,
            self.source,
            classeur::ordre_du_classeur(&chemin, config.ordre_tri()),
            &self.priorites,
        ) {
            self.toutes = cartes;
            self.raretes = classeur::raretes_disponibles(&self.toutes);
            self.refiltrer();
        }
    }

    /// Navigation et mesures.
    fn barre_du_bas(&mut self, racine: &mut egui::Ui) {
        let total = classeur::nb_doubles_pages(self.visibles.len(), self.grille);
        egui::Panel::bottom("navigation").show(racine, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                // ── La navigation, au centre ────────────────────────────
                //
                // Elle était collée à gauche, là où l'œil ne la cherche pas
                // sur un écran large : deux mille cinq cents points séparaient
                // « Suivant » du bord droit. On la centre.
                //
                // La largeur du groupe est celle **mesurée à l'image
                // précédente** : egui ne la connaît qu'après l'avoir dessiné,
                // et une estimation en dur dériverait au premier libellé
                // changé. Le décalage d'une image est invisible ; au premier
                // rendu le groupe part de la gauche, le temps d'une frame.
                ui.add_space(centrage(ui.available_width(), self.largeur_navigation));
                let debut = ui.cursor().min.x;

                if ui
                    .add_enabled(self.double_page > 0, egui::Button::new("◀ Précédent"))
                    .clicked()
                {
                    self.double_page -= 1;
                }
                let (gauche, droite) = classeur::numeros_de_page(self.double_page);
                let pages = match gauche {
                    Some(g) => format!("pages {g} – {droite}"),
                    None => format!("page {droite}"),
                };
                ui.label(format!(
                    "{pages}   ·   feuillet {} / {total}",
                    self.double_page + 1
                ));
                if ui
                    .add_enabled(
                        self.double_page + 1 < total,
                        egui::Button::new("Suivant ▶"),
                    )
                    .clicked()
                {
                    self.double_page += 1;
                }

                // ── Le saut de page ─────────────────────────────────────
                //
                // Quarante feuillets pour `RA05` : atteindre le trente-
                // septième au bouton demande trente-six clics.
                if total > 1 {
                    ui.add_space(16.0);
                    ui.label("Aller à la page");
                    let derniere = classeur::derniere_page(total);
                    let champ = ui.add(
                        egui::DragValue::new(&mut self.page_visee)
                            .range(1..=derniere)
                            .speed(0.2),
                    );
                    let entree = champ.lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if ui.button("Aller").clicked() || champ.drag_stopped() || entree {
                        self.double_page = classeur::feuillet_de_page(self.page_visee, total);
                    }
                }

                self.largeur_navigation = ui.cursor().min.x - debut;

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let fiche = if self.duree_fiche == Duration::ZERO {
                        String::new()
                    } else {
                        format!(" · fiche {:?}", self.duree_fiche)
                    };
                    ui.small(format!(
                        "chargement {:?} · filtre {:?}{fiche} · {} texture(s), {} décodage(s) en {:?}, {} image(s) absente(s)",
                        self.duree_chargement,
                        self.duree_filtre,
                        self.cache.taille(),
                        self.cache.decodages(),
                        self.cache.duree_decodage(),
                        self.cache.manquants(),
                    ));
                });
            });
            ui.add_space(6.0);
        });
    }

    /// Les deux pages, côte à côte.
    fn pages(&mut self, racine: &mut egui::Ui) {
        egui::CentralPanel::default().show(racine, |ui| {
            // Un classeur filtré jusqu'au vide affichait des cases vides, que
            // rien ne distinguait d'un feuillet non rempli : l'utilisateur en
            // a conclu que la recherche était cassée. On le dit.
            if self.visibles.is_empty() {
                self.rien_a_montrer(ui);
                return;
            }
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let fenetre = classeur::Fenetre {
                largeur: ui.available_width().max(0.0) as u32,
                hauteur: ui.available_height().max(0.0) as u32,
            };
            let (largeur, hauteur) = classeur::taille_carte(fenetre, self.grille, BORNES);
            let (gauche, droite) =
                classeur::double_page(&self.visibles, self.double_page, self.grille);
            let gauche = gauche.to_vec();
            let droite = droite.to_vec();

            // ── Le centrage ────────────────────────────────────────────
            //
            // Une double page 3×3 a un rapport largeur/hauteur d'environ 1,37
            // quand un écran 16:9 en a 1,78 : même parfaitement ajustée, elle
            // ne remplira jamais la largeur. Ce reste-là est géométrique, il
            // n'est pas un défaut — mais il doit se répartir des deux côtés,
            // sans quoi le classeur reste collé à gauche.
            #[allow(clippy::cast_precision_loss)]
            let occupe = classeur::largeur_double_page(largeur, self.grille) as f32;
            let marge = ((ui.available_width() - occupe) / 2.0).max(12.0);

            // Ces trois espaces sont ceux que `taille_carte` a retranchés
            // pour calculer la taille des cartes. Les écrire en clair ici
            // les ferait diverger au premier réglage : le calcul et le
            // dessin doivent puiser à la même source, sinon la double page
            // déborde exactement de la différence.
            #[allow(clippy::cast_precision_loss)]
            let marge_verticale = classeur::MARGE_VERTICALE as f32;
            #[allow(clippy::cast_precision_loss)]
            let entre_pages = classeur::ESPACE_ENTRE_PAGES as f32;
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(marge_verticale);
                ui.horizontal_top(|ui| {
                    ui.add_space(marge);
                    self.page(ui, &gauche, largeur, hauteur);
                    ui.add_space(entre_pages);
                    self.page(ui, &droite, largeur, hauteur);
                });
                ui.add_space(marge_verticale);
            });
        });
    }

    /// Ce qui s'affiche quand aucune carte ne passe les filtres.
    ///
    /// Le message nomme le filtre fautif plutôt que de constater le vide : sur
    /// `RA02`, chercher `080` ne ramène rien parce que le set s'arrête à
    /// `079`, et non parce qu'il faudrait saisir le code entier.
    fn rien_a_montrer(&mut self, ui: &mut egui::Ui) {
        let cause = if !self.filtres.terme.trim().is_empty() {
            format!(
                "Aucune carte ne correspond à « {} ».",
                self.filtres.terme.trim()
            )
        } else if let Some(rarete) = &self.filtres.rarete {
            format!("Aucune carte en {rarete}.")
        } else {
            match self.filtres.possession {
                Possession::Possedees => "Aucune carte possédée dans ce classeur.".to_owned(),
                Possession::NonPossedees => "Toutes les cartes sont possédées.".to_owned(),
                Possession::Toutes => "Ce classeur est vide.".to_owned(),
            }
        };
        ui.vertical_centered(|ui| {
            ui.add_space(96.0);
            ui.heading(cause);
            ui.add_space(8.0);
            ui.label(format!(
                "{} carte(s) dans le classeur. Échap efface la recherche.",
                self.toutes.len()
            ));
            ui.add_space(12.0);
            if ui.button("Effacer les filtres").clicked() {
                self.filtres.terme.clear();
                self.filtres.rarete = None;
                self.filtres.possession = Possession::Toutes;
                self.refiltrer();
            }
        });
    }

    /// Une page : la grille de cases, cartes puis emplacements vides.
    fn page(&mut self, ui: &mut egui::Ui, cartes: &[Carte], largeur: u32, hauteur: u32) {
        let grille = self.grille;
        disposer_page(ui, grille, |ui, i| match cartes.get(i) {
            Some(carte) => self.case(ui, carte, largeur, hauteur),
            None => case_vide(ui, largeur, hauteur),
        });
    }

    /// Une case occupée par une carte.
    ///
    /// Toutes les commandes de quantité sont ici : les deux boutons, la
    /// molette, et le menu contextuel. Aucune n'écrit directement — elles
    /// empilent une [`Action`] appliquée après le dessin.
    fn case(&mut self, ui: &mut egui::Ui, carte: &Carte, largeur: u32, hauteur: u32) {
        #[allow(clippy::cast_precision_loss)]
        let taille = egui::vec2(largeur as f32, hauteur as f32);
        let (rect, reponse) = ui.allocate_exact_size(taille, egui::Sense::click());
        let survolee = reponse.hovered();
        if survolee {
            self.survolee = Some(carte.rowid);
        }
        // Le clic gauche était la seule place libre : le survol arme déjà la
        // molette, le clic droit ouvre le menu. Le Python ouvrait lui aussi
        // son dialogue au clic sur la carte.
        if reponse.clicked() {
            self.en_attente.push(Action::OuvrirFiche(carte.rowid));
        }
        let possedee = carte.quantite > 0;

        // ── L'illustration ──────────────────────────────────────────────
        let texture = carte.fichier_image.as_ref().and_then(|fichier| {
            self.cache.texture(
                ui.ctx(),
                &self.dossier_images(),
                Cle::nouvelle(fichier, possedee, survolee),
            )
        });
        match texture {
            Some(texture) => {
                egui::Image::from_texture(&texture)
                    .corner_radius(4.0)
                    .paint_at(ui, rect);
            }
            None => {
                ui.painter()
                    .rect_filled(rect, 4.0, ui.visuals().extreme_bg_color);
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    &carte.set_code,
                    egui::FontId::proportional(11.0),
                    ui.visuals().weak_text_color(),
                );
            }
        }

        // ── Le repère de la carte trouvée ───────────────────────────────
        //
        // Une proposition de recherche envoie à la page ; sans repère,
        // l'utilisateur arrive devant neuf cartes sans savoir laquelle il
        // cherchait. Le cadre d'or s'efface tout seul au bout de
        // `DUREE_SURLIGNAGE`, sans quoi il faudrait un geste pour s'en
        // débarrasser.
        if let Some((rowid, pose)) = self.surlignee {
            if rowid == carte.rowid {
                let age = ui.input(|i| i.time) - pose;
                if age < DUREE_SURLIGNAGE {
                    // L'opacité décroît sur la fin : le repère s'éteint au lieu
                    // de disparaître d'un coup.
                    #[allow(clippy::cast_possible_truncation)]
                    let reste =
                        ((DUREE_SURLIGNAGE - age) / DUREE_SURLIGNAGE).clamp(0.0, 1.0) as f32;
                    let couleur = COULEUR_TROUVEE.gamma_multiply(opacite_du_repere(reste));
                    ui.painter().rect_stroke(
                        rect.expand(2.0),
                        6.0,
                        egui::Stroke::new(3.0, couleur),
                        egui::StrokeKind::Outside,
                    );
                    // Tant que le repère vit, l'écran se redessine : sinon
                    // l'extinction n'aurait lieu qu'au prochain mouvement de
                    // souris.
                    ui.ctx().request_repaint();
                } else {
                    self.surlignee = None;
                }
            }
        }

        // ── La molette : ±1 sans un seul clic ───────────────────────────
        //
        // C'est la commande la plus rapide pour saisir une collection : on
        // balaie la page à la souris sans jamais viser un bouton.
        if survolee {
            // Le défilement est CONSOMMÉ : sans cela la molette changerait la
            // quantité **et** ferait défiler la double page en même temps, ce
            // qui rendrait la commande inutilisable. La `ScrollArea` qui
            // entoure les pages lit ce même delta après le dessin des cartes.
            let crans = ui.ctx().input_mut(|i| {
                let delta = i.smooth_scroll_delta.y;
                if delta.abs() > 0.5 {
                    i.smooth_scroll_delta = egui::Vec2::ZERO;
                }
                delta
            });
            if crans.abs() > 0.5 {
                self.en_attente.push(Action::Ajuster {
                    rowid: carte.rowid,
                    delta: if crans > 0.0 { 1 } else { -1 },
                });
            }
        }

        // ── Les deux boutons d'incrément, aux coins hauts ───────────────
        let bouton = egui::vec2(24.0, 24.0);
        let marge = 3.0;
        let moins = egui::Rect::from_min_size(rect.left_top() + egui::vec2(marge, marge), bouton);
        let plus = egui::Rect::from_min_size(
            rect.right_top() + egui::vec2(-marge - bouton.x, marge),
            bouton,
        );

        if self.incrementer(ui, moins, "−", possedee, survolee) {
            self.en_attente.push(Action::Ajuster {
                rowid: carte.rowid,
                delta: -1,
            });
        }
        if self.incrementer(ui, plus, "+", true, survolee) {
            self.en_attente.push(Action::Ajuster {
                rowid: carte.rowid,
                delta: 1,
            });
        }

        // ── La quantité, au centre entre les deux boutons ───────────────
        //
        // Son coin d'origine — en haut à droite dans la V1 — est pris par le
        // « + ». Le centre haut la garde lisible et à distance des commandes.
        if possedee {
            let pastille = egui::Rect::from_center_size(
                egui::pos2(rect.center().x, rect.top() + marge + bouton.y / 2.0),
                egui::vec2(34.0, 20.0),
            );
            ui.painter()
                .rect_filled(pastille, 10.0, egui::Color32::from_black_alpha(200));
            ui.painter().text(
                pastille.center(),
                egui::Align2::CENTER_CENTER,
                format!("×{}", carte.quantite),
                egui::FontId::proportional(12.0),
                COULEUR_OR,
            );
        }

        // ── Les badges du bas, à leur place d'origine ───────────────────
        if carte.quantite >= ygo_app::possession::SEUIL_PLAYSET {
            badge(ui, rect.left_bottom(), false, "PLAYSET", COULEUR_PLAYSET);
        }
        if carte.extended_art {
            badge(
                ui,
                rect.right_bottom(),
                true,
                "OVERFRAME",
                COULEUR_OVERFRAME,
            );
        }

        // ── Le bandeau de survol ────────────────────────────────────────
        if survolee {
            let bandeau = egui::Rect::from_min_max(
                egui::pos2(rect.left(), rect.bottom() - 62.0),
                rect.right_bottom(),
            );
            ui.painter()
                .rect_filled(bandeau, 4.0, egui::Color32::from_black_alpha(205));
            let mut haut = bandeau.left_top() + egui::vec2(6.0, 4.0);
            for (texte, taille) in [
                (carte.nom.as_str(), 11.0),
                (carte.set_code.as_str(), 10.0),
                (carte.rarete.as_str(), 10.0),
            ] {
                ui.painter().text(
                    haut,
                    egui::Align2::LEFT_TOP,
                    texte,
                    egui::FontId::proportional(taille),
                    egui::Color32::from_gray(220),
                );
                haut.y += taille + 4.0;
            }
        }

        // ── Le menu contextuel ──────────────────────────────────────────
        self.menu(&reponse, carte);
    }

    /// Un bouton d'incrément. Rend `true` s'il faut agir.
    ///
    /// Il se répète tant qu'on le maintient enfoncé : passer de 0 à 3 est un
    /// geste, pas trois clics. Le premier pas est immédiat, les suivants
    /// arrivent après un délai puis à cadence fixe — la convention de tous les
    /// sélecteurs de nombre.
    fn incrementer(
        &mut self,
        ui: &mut egui::Ui,
        zone: egui::Rect,
        signe: &str,
        actif: bool,
        survolee: bool,
    ) -> bool {
        let id = ui
            .id()
            .with((signe, zone.left_top().x as i32, zone.top() as i32));
        let reponse = ui.interact(zone, id, egui::Sense::click_and_drag());
        let vif = reponse.hovered() || survolee;

        // Discret au repos, franc au survol : présent sans encombrer.
        let fond = if !actif {
            egui::Color32::from_black_alpha(60)
        } else if reponse.hovered() {
            COULEUR_OR
        } else if vif {
            egui::Color32::from_black_alpha(190)
        } else {
            egui::Color32::from_black_alpha(110)
        };
        let encre = if reponse.hovered() && actif {
            egui::Color32::BLACK
        } else if actif {
            egui::Color32::from_gray(235)
        } else {
            egui::Color32::from_gray(110)
        };
        ui.painter().rect_filled(zone, 12.0, fond);
        ui.painter().text(
            zone.center(),
            egui::Align2::CENTER_CENTER,
            signe,
            egui::FontId::proportional(16.0),
            encre,
        );

        if !actif {
            return false;
        }
        if reponse.clicked() {
            return true;
        }
        // La répétition, tant que le bouton reste enfoncé.
        if reponse.is_pointer_button_down_on() {
            let maintenu = ui
                .ctx()
                .input(|i| i.pointer.press_start_time())
                .map_or(0.0, |debut| ui.ctx().input(|i| i.time) - debut);
            if maintenu > DELAI_REPETITION {
                ui.ctx().request_repaint();
                let periode = 1.0 / CADENCE_REPETITION;
                let pas = ((maintenu - DELAI_REPETITION) / periode).floor();
                let precedent = self.dernier_pas.get(&id).copied().unwrap_or(-1.0);
                if pas > precedent {
                    self.dernier_pas.insert(id, pas);
                    return true;
                }
            }
        } else {
            self.dernier_pas.remove(&id);
        }
        false
    }

    /// Le menu contextuel d'une carte.
    ///
    /// Les entrées que le portage ne sait pas encore servir sont **montrées
    /// mais désactivées**, avec la raison en infobulle : les cacher donnerait
    /// l'illusion d'un menu complet.
    fn menu(&mut self, reponse: &egui::Response, carte: &Carte) {
        let rowid = carte.rowid;
        let nom = carte.nom.clone();
        let code = carte.set_code.clone();
        let quantite = carte.quantite;
        reponse.context_menu(|ui| {
            ui.label(egui::RichText::new(&nom).strong());
            ui.label(egui::RichText::new(&code).small().weak());
            ui.separator();

            if ui
                .add_enabled(quantite > 0, egui::Button::new("Remettre à zéro"))
                .clicked()
            {
                self.en_attente.push(Action::RemettreAZero { rowid });
                ui.close();
            }
            if ui.button("Marquer comme possédée (×1)").clicked() {
                self.en_attente.push(Action::Ajuster {
                    rowid,
                    delta: 1 - quantite,
                });
                ui.close();
            }
            ui.separator();
            if ui.button("Copier le nom").clicked() {
                ui.ctx().copy_text(nom.clone());
                ui.close();
            }
            if ui.button("Copier le numéro").clicked() {
                ui.ctx().copy_text(code.clone());
                ui.close();
            }
            ui.separator();
            if ui
                .button("Modifier l'artwork…")
                .on_hover_text("Les autres illustrations connues pour cette carte")
                .clicked()
            {
                self.artworks = Some(code.clone());
                ui.close();
            }
            ui.add_enabled(false, egui::Button::new("Qualité…"))
                .on_disabled_hover_text("le dialogue de carte n'est pas encore porté");
        });
    }
}

/// Doré des quantités — `C["gold"]`.
const COULEUR_OR: egui::Color32 = egui::Color32::from_rgb(212, 175, 55);
/// Violet du badge Overframe — `C["overframe"]`.
const COULEUR_OVERFRAME: egui::Color32 = egui::Color32::from_rgb(138, 92, 200);
/// Vert du badge Playset — `C["playset"]`.
const COULEUR_PLAYSET: egui::Color32 = egui::Color32::from_rgb(64, 150, 108);
/// Cyan du repère posé sur la carte qu'une proposition vient de montrer.
///
/// Volontairement distinct de [`COULEUR_OR`] : le doré dit « tu la possèdes »,
/// celui-ci dit « c'est celle-là que tu cherchais ». Deux messages, deux
/// couleurs.
const COULEUR_TROUVEE: egui::Color32 = egui::Color32::from_rgb(90, 200, 250);

/// Délai avant que le maintien d'un bouton ne se mette à répéter, en secondes.
const DELAI_REPETITION: f64 = 0.4;
/// Cadence de la répétition, en pas par seconde.
const CADENCE_REPETITION: f64 = 8.0;

/// Durée du repère posé sur la carte trouvée, en secondes.
const DUREE_SURLIGNAGE: f64 = 3.0;

/// Part de la durée pendant laquelle le repère reste pleinement opaque.
const PALIER_REPERE: f32 = 0.4;

/// L'opacité du repère, en fonction de ce qu'il lui reste à vivre.
///
/// Pleine sur les 60 premiers pour cent de sa durée, puis décroissante. Un
/// fondu qui commence tout de suite se lit comme un défaut d'affichage ; un
/// repère qui disparaît d'un coup se lit comme un clignotement. Le palier
/// donne le temps de voir la carte, la pente dit que ça s'en va.
///
/// `reste` va de 1 (posé à l'instant) à 0 (expiré).
#[must_use]
fn opacite_du_repere(reste: f32) -> f32 {
    if reste >= PALIER_REPERE {
        1.0
    } else {
        (reste / PALIER_REPERE).clamp(0.0, 1.0)
    }
}

/// Un badge d'angle, en bas de la carte.
fn badge(ui: &egui::Ui, coin: egui::Pos2, a_droite: bool, texte: &str, fond: egui::Color32) {
    let police = egui::FontId::proportional(9.0);
    let largeur = texte.len() as f32 * 5.6 + 10.0;
    let taille = egui::vec2(largeur, 15.0);
    let min = if a_droite {
        coin + egui::vec2(-taille.x - 4.0, -taille.y - 4.0)
    } else {
        coin + egui::vec2(4.0, -taille.y - 4.0)
    };
    let zone = egui::Rect::from_min_size(min, taille);
    ui.painter().rect_filled(zone, 3.0, fond);
    ui.painter().text(
        zone.center(),
        egui::Align2::CENTER_CENTER,
        texte,
        police,
        egui::Color32::WHITE,
    );
}

/// Ce qu'une commande de l'utilisateur demande.
///
/// Les actions sont mises en attente pendant le dessin puis appliquées après :
/// modifier la liste des cartes au milieu de son parcours n'est pas possible,
/// et le faire par un drapeau dispersé serait pire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Ajouter `delta` à la quantité d'une carte.
    Ajuster {
        /// La ligne visée.
        rowid: i64,
        /// Le pas, positif ou négatif.
        delta: i64,
    },
    /// Remettre la quantité d'une carte à zéro.
    RemettreAZero {
        /// La ligne visée.
        rowid: i64,
    },
    /// Basculer **tout** le classeur — cf. [`ygo_app::possession::Bascule`].
    ///
    /// Différée comme les autres, et suivie d'un rechargement complet : elle
    /// touche des centaines de lignes, mettre la liste à jour une par une
    /// n'aurait aucun intérêt.
    Basculer(ygo_app::possession::Bascule),
    /// Ouvrir la fiche d'une carte.
    ///
    /// Différée comme les autres : elle lit la base, et le faire au milieu du
    /// dessin de la page reviendrait à muter l'écran qu'on est en train de
    /// parcourir.
    OuvrirFiche(i64),
}

/// Dispose les cases d'une page en `lignes` rangées de `colonnes`.
///
/// # Le `vertical` n'est pas décoratif
///
/// Les deux pages sont posées dans un `horizontal_top`, et le cadre de chaque
/// page **hérite de cette direction**. Sans le `vertical` explicite, chaque
/// rangée se place à côté de la précédente : une page 3×3 s'affiche sur une
/// seule ligne de neuf cases, et la seconde page part hors de l'écran.
///
/// Le bug a été livré une fois. Cette fonction existe pour qu'il soit
/// **testable sans fenêtre** — cf. les tests du module.
pub fn disposer_page(
    ui: &mut egui::Ui,
    grille: Grille,
    mut dessiner: impl FnMut(&mut egui::Ui, usize),
) {
    let colonnes = usize::from(grille.colonnes).max(1);
    let cases = grille.par_page();
    // Le rembourrage et l'écart entre cases sont ceux que `taille_carte` a
    // retranchés. Deux littéraux au lieu des constantes, et la double page
    // déborde de la différence dès qu'on règle l'un des deux.
    #[allow(clippy::cast_possible_truncation)]
    let rembourrage = classeur::PADDING_PAGE as i8;
    #[allow(clippy::cast_precision_loss)]
    let ecart = classeur::ESPACE_CARTES as f32;
    egui::Frame::group(ui.style())
        .corner_radius(12.0)
        .inner_margin(rembourrage)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(ecart, ecart);
            ui.vertical(|ui| {
                for debut in (0..cases).step_by(colonnes) {
                    ui.horizontal(|ui| {
                        for i in debut..(debut + colonnes).min(cases) {
                            dessiner(ui, i);
                        }
                    });
                }
            });
        });
}

/// De combien décaler un groupe de `largeur` pour le centrer dans `disponible`.
///
/// Rend zéro plutôt qu'un décalage négatif : un groupe plus large que la place
/// disponible doit commencer au bord, pas déborder à gauche.
///
/// ```
/// use ygo_ui::classeur::centrage;
/// assert_eq!(centrage(1000.0, 400.0), 300.0);
/// assert_eq!(centrage(300.0, 400.0), 0.0, "plus large que la place");
/// assert_eq!(centrage(1000.0, 0.0), 0.0, "pas encore mesuré : on ne bouge pas");
/// ```
#[must_use]
pub fn centrage(disponible: f32, largeur: f32) -> f32 {
    if largeur <= 0.0 {
        return 0.0;
    }
    ((disponible - largeur) / 2.0).max(0.0)
}

/// Une case sans carte — le classeur en montre toujours `colonnes × lignes`.
fn case_vide(ui: &mut egui::Ui, largeur: u32, hauteur: u32) {
    #[allow(clippy::cast_precision_loss)]
    let taille = egui::vec2(largeur as f32, hauteur as f32);
    let (rect, _) = ui.allocate_exact_size(taille, egui::Sense::hover());
    ui.painter().rect_stroke(
        rect,
        4.0,
        egui::Stroke::new(1.0, ui.visuals().weak_text_color()),
        egui::StrokeKind::Inside,
    );
}

/// Les facteurs employés, exposés pour la documentation de l'écran.
#[must_use]
pub fn facteurs(carte: &Carte, survolee: bool) -> Facteurs {
    Facteurs::pour(carte.quantite > 0, survolee)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    /// Déroule une image d'interface **sans fenêtre** et rend les rectangles
    /// alloués, dans l'ordre.
    ///
    /// egui sait tourner en pur calcul : c'est ce qui permet d'éprouver une
    /// disposition sans écran, là où l'apparence, elle, restera toujours du
    /// ressort de l'œil.
    fn rectangles(grille: Grille, taille: egui::Vec2) -> Vec<egui::Rect> {
        let ctx = egui::Context::default();
        let vus = std::cell::RefCell::new(Vec::new());
        // egui 0.36 : `run_ui` remplace `run` et donne directement le `Ui`
        // racine — le même changement que pour `App::ui`.
        let mut sortie = ctx.run_ui(egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                // Le `horizontal_top` n'est PAS décoratif : c'est lui qui
                // reproduit la condition du bug. Sans lui, la page hérite du
                // panneau — vertical — et le défaut ne se manifeste jamais.
                // Première version de ce banc d'essai : elle passait avec ET
                // sans le correctif, donc elle ne prouvait rien.
                ui.horizontal_top(|ui| {
                    disposer_page(ui, grille, |ui, _i| {
                        let (rect, _) = ui.allocate_exact_size(taille, egui::Sense::hover());
                        vus.borrow_mut().push(rect);
                    });
                });
            });
        });
        // Sans cela, `FullOutput` panique en se libérant : egui exige que les
        // deltas de textures soient traités. Hors fenêtre, il n'y a rien à en
        // faire — on les écarte explicitement.
        sortie.textures_delta.clear();
        vus.into_inner()
    }

    #[test]
    fn une_page_a_exactement_colonnes_fois_lignes_cases() {
        let taille = egui::vec2(60.0, 88.0);
        assert_eq!(
            rectangles(
                Grille {
                    colonnes: 3,
                    lignes: 3
                },
                taille
            )
            .len(),
            9
        );
        assert_eq!(
            rectangles(
                Grille {
                    colonnes: 4,
                    lignes: 3
                },
                taille
            )
            .len(),
            12
        );
        assert_eq!(
            rectangles(
                Grille {
                    colonnes: 4,
                    lignes: 4
                },
                taille
            )
            .len(),
            16
        );
    }

    #[test]
    fn le_repere_reste_opaque_puis_s_eteint() {
        assert_eq!(opacite_du_repere(1.0), 1.0, "posé à l'instant");
        assert_eq!(opacite_du_repere(0.5), 1.0, "encore dans le palier");
        assert_eq!(opacite_du_repere(PALIER_REPERE), 1.0, "juste au palier");
        assert!(
            opacite_du_repere(PALIER_REPERE / 2.0) < 1.0,
            "la pente a commencé"
        );
        assert_eq!(opacite_du_repere(0.0), 0.0, "éteint");
        // Monotone : le repère ne se rallume jamais.
        let mut precedente = 0.0;
        for pas in 0..=100 {
            #[allow(clippy::cast_precision_loss)]
            let o = opacite_du_repere(pas as f32 / 100.0);
            assert!(o >= precedente, "l'opacité remonte à {pas}");
            precedente = o;
        }
    }

    #[test]
    fn les_rangees_s_empilent_au_lieu_de_se_suivre() {
        // LE bug livré une fois : les neuf cases d'une page 3×3 se sont
        // retrouvées sur une seule ligne, parce que le cadre héritait de la
        // direction horizontale du spread.
        let taille = egui::vec2(60.0, 88.0);
        let rects = rectangles(
            Grille {
                colonnes: 3,
                lignes: 3,
            },
            taille,
        );

        #[allow(clippy::cast_possible_truncation)]
        let hauts: std::collections::BTreeSet<i64> =
            rects.iter().map(|r| r.top().round() as i64).collect();
        assert_eq!(hauts.len(), 3, "trois rangées distinctes, pas une");

        #[allow(clippy::cast_possible_truncation)]
        let gauches: std::collections::BTreeSet<i64> =
            rects.iter().map(|r| r.left().round() as i64).collect();
        assert_eq!(gauches.len(), 3, "trois colonnes distinctes");
    }

    #[test]
    fn les_cases_sont_remplies_de_gauche_a_droite_puis_de_haut_en_bas() {
        let taille = egui::vec2(60.0, 88.0);
        let rects = rectangles(
            Grille {
                colonnes: 3,
                lignes: 2,
            },
            taille,
        );
        assert_eq!(rects.len(), 6);

        // Les trois premières partagent leur bord haut et avancent vers la
        // droite ; la quatrième redescend et repart à gauche.
        assert!((rects[0].top() - rects[2].top()).abs() < 0.5);
        assert!(rects[0].left() < rects[1].left());
        assert!(rects[1].left() < rects[2].left());
        assert!(rects[3].top() > rects[0].top(), "la seconde rangée descend");
        assert!(
            (rects[3].left() - rects[0].left()).abs() < 0.5,
            "et repart de la même colonne"
        );
    }

    /// Ce que la double page occupe **réellement**, mesuré, pour une fenêtre
    /// donnée : les deux pages, leurs cadres, l'espace qui les sépare et les
    /// marges que l'écran ajoute au-dessus et au-dessous.
    ///
    /// La mesure passe par un `Context` de la taille de la fenêtre et par le
    /// vrai `disposer_page`, pas par l'arithmétique de `taille_carte` — sans
    /// quoi le test confronterait le calcul à lui-même.
    fn etendue_double_page(grille: Grille, fenetre: classeur::Fenetre) -> egui::Vec2 {
        let (largeur, hauteur) = classeur::taille_carte(fenetre, grille, BORNES);
        #[allow(clippy::cast_precision_loss)]
        let taille = egui::vec2(largeur as f32, hauteur as f32);
        let ctx = egui::Context::default();
        #[allow(clippy::cast_precision_loss)]
        let entree = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(fenetre.largeur as f32, fenetre.hauteur as f32),
            )),
            ..Default::default()
        };
        let mesure = std::cell::Cell::new(egui::Vec2::ZERO);
        let mut sortie = ctx.run_ui(entree, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                let dessiner = |ui: &mut egui::Ui| {
                    disposer_page(ui, grille, |ui, _i| {
                        let _ = ui.allocate_exact_size(taille, egui::Sense::hover());
                    });
                };
                // La disposition de l'écran, à l'identique : marge, deux
                // pages côte à côte, marge.
                #[allow(clippy::cast_precision_loss)]
                let marge = classeur::MARGE_VERTICALE as f32;
                ui.add_space(marge);
                let rect = ui
                    .horizontal_top(|ui| {
                        dessiner(ui);
                        #[allow(clippy::cast_precision_loss)]
                        ui.add_space(classeur::ESPACE_ENTRE_PAGES as f32);
                        dessiner(ui);
                    })
                    .response
                    .rect;
                ui.add_space(marge);
                mesure.set(rect.size() + egui::vec2(0.0, 2.0 * marge));
            });
        });
        // egui exige que les deltas de textures soient traités ; hors fenêtre
        // il n'y a rien à en faire.
        sortie.textures_delta.clear();
        mesure.get()
    }

    /// La double page tient dans la fenêtre, **sur les deux axes**, mesurée.
    ///
    /// # Ce que la version précédente ne voyait pas
    ///
    /// Elle mesurait l'étendue horizontale des cases d'une seule page, la
    /// doublait, et comparait à 1 400. Deux angles morts : les cadres et
    /// l'espace entre les pages n'y étaient pas, et **la hauteur n'y était
    /// pas du tout** — c'est précisément l'axe que le calcul ignorait. Sur
    /// 2 560 × 1 440, une taille calculée sur la seule largeur réclamait
    /// 1 692 points de haut : la double page passait sous la barre de
    /// défilement, et ce test-là restait vert.
    #[test]
    fn deux_pages_tiennent_dans_la_fenetre() {
        for grille in [
            Grille {
                colonnes: 3,
                lignes: 3,
            },
            Grille {
                colonnes: 4,
                lignes: 3,
            },
            Grille {
                colonnes: 3,
                lignes: 4,
            },
        ] {
            // Deux familles de fenêtres, et il faut les deux : sur un 16:9
            // c'est la hauteur qui limite et l'arithmétique de largeur n'est
            // jamais mise à l'épreuve. Les deux dernières sont hautes et
            // étroites, là où c'est la largeur qui décide.
            for (largeur, hauteur) in [
                (1400, 900),
                (1920, 1080),
                (2560, 1440),
                (3840, 2160),
                (1100, 1800),
                (1000, 2400),
            ] {
                let fenetre = classeur::Fenetre { largeur, hauteur };
                let occupe = etendue_double_page(grille, fenetre);
                #[allow(clippy::cast_precision_loss)]
                let (l, h) = (largeur as f32, hauteur as f32);
                assert!(
                    occupe.x <= l,
                    "{grille:?} en {largeur}×{hauteur} : {} points de large",
                    occupe.x
                );
                assert!(
                    occupe.y <= h,
                    "{grille:?} en {largeur}×{hauteur} : {} points de haut",
                    occupe.y
                );
            }
        }
    }

    /// Et elle occupe vraiment la hauteur qu'on lui laisse.
    ///
    /// Le pendant du test précédent : « ça tient » est trivialement vrai pour
    /// une carte de 90 points. Ce qui est demandé, c'est que la double page
    /// **remplisse** l'écran — au moins les trois quarts de sa hauteur, sur
    /// un 16:9 où c'est la hauteur qui limite.
    #[test]
    fn deux_pages_remplissent_la_hauteur_disponible() {
        let grille = Grille {
            colonnes: 3,
            lignes: 3,
        };
        for (largeur, hauteur) in [(1920, 1080), (2560, 1440)] {
            let occupe = etendue_double_page(grille, classeur::Fenetre { largeur, hauteur });
            #[allow(clippy::cast_precision_loss)]
            let attendu = hauteur as f32 * 0.75;
            assert!(
                occupe.y >= attendu,
                "en {largeur}×{hauteur}, la double page n'occupe que {} points de haut",
                occupe.y
            );
        }
    }

    #[test]
    fn une_grille_degeneree_ne_boucle_pas_indefiniment() {
        let rects = rectangles(
            Grille {
                colonnes: 0,
                lignes: 3,
            },
            egui::vec2(60.0, 88.0),
        );
        assert!(rects.is_empty(), "aucune case, et surtout aucune boucle");
    }
}
