// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! L'écran des options.
//!
//! Portage de `ui/ecran_options.py`, moins ce qui n'est pas encore porté
//! ailleurs — l'écran Python fait 1 472 lignes et pilote aussi des modules
//! absents de ce portage.
//!
//! # Rien n'est décidé ici
//!
//! Chaque réglage a son écriture typée dans [`ygo_core::config`], et **chacune
//! rend la valeur effective** — celle qui a réellement été enregistrée après
//! bornage. L'écran resynchronise ses champs sur cette valeur plutôt que sur
//! ce que l'utilisateur a tapé : sans cela, une case afficherait `99` là où
//! `10` a été écrit.
//!
//! Les priorités de rareté sont l'exception : leur table vit dans
//! `rarity_config.json`, pas dans `app_config.json`, et [`Priorites`] a sa
//! propre écriture.
//!
//! # Ce qui change tout de suite, et ce qui attend
//!
//! Langue, source d'images, ordre de tri et N raretés changent ce que l'écran
//! classeur **calcule** : ils demandent une relecture. L'écran signale donc à
//! l'application ce qu'il a touché ; c'est elle qui décide de rouvrir le
//! classeur ou non — la même séparation qu'ailleurs.

use eframe::egui;
use ygo_app::maj::{EtatLocal, Verdict};
use ygo_core::config::{
    Config, CritereTri, Langue, SourceImage, FONT_SCALE_MAX, FONT_SCALE_MIN, GRILLE_MAX,
    GRILLE_MIN, N_RARETES_MAX, N_RARETES_MIN,
};
use ygo_core::paths::Paths;
use ygo_core::rarity::Priorites;

/// Ce qu'un changement de réglage oblige à refaire.
///
/// Un simple drapeau ne suffirait pas : changer la source d'images ne demande
/// pas le même travail que changer l'ordre de tri, et tout recharger à chaque
/// case cochée rendrait l'écran poussif.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Consequences {
    /// Les cartes doivent être relues et retriées — langue, ordre, priorités.
    pub relire_classeur: bool,
    /// Le cache de textures est périmé — la source d'images a changé.
    pub vider_images: bool,
    /// La liste des classeurs doit être relue — grille par défaut.
    pub relire_accueil: bool,
}

impl Consequences {
    /// Y a-t-il quelque chose à faire ?
    #[must_use]
    pub fn quelque_chose(self) -> bool {
        self.relire_classeur || self.vider_images || self.relire_accueil
    }

    /// Fusionne deux jeux de conséquences.
    #[must_use]
    pub fn avec(self, autre: Self) -> Self {
        Self {
            relire_classeur: self.relire_classeur || autre.relire_classeur,
            vider_images: self.vider_images || autre.vider_images,
            relire_accueil: self.relire_accueil || autre.relire_accueil,
        }
    }
}

/// Ce qu'une commande de la base fait, et sous quel nom.
///
/// # Deux commandes, pas un bouton à trois humeurs
///
/// Première version de ce portage : **un** bouton, dont le libellé changeait
/// selon le verdict — « Mettre à jour » devenant « Reconstruire » quand on
/// avait déjà la dernière version. L'utilisateur a signalé que le Python
/// faisait autre chose, et la source lui donne raison. Il en avait **deux** :
///
/// - `ui/update_window.py` — la MAJ, atteinte depuis l'accueil. Quand les
///   versions coïncident, elle affiche « ✅ La base de données est à jour » et
///   **un seul bouton, Fermer**. Rien à forcer. Elle sauvegarde, appelle
///   `run_init`, puis écrit `last_update.txt`.
/// - `ui/ecran_options.py` §Maintenance — « 🔧 Initialiser la base », sans
///   verdict ni condition, dans une section dont le sous-titre dit « À
///   utiliser en cas de problème ». Elle appelle `run_init` seul.
///
/// La distinction n'est pas cosmétique : ce sont deux intentions. « Ai-je du
/// retard ? » se répond par un verdict, et un non ferme la question. « Ma base
/// est-elle abîmée ? » ne se répond par aucun verdict — l'épisode des raretés
/// numériques du 2026-09-01 l'a montré, rien dans `146.68` ne disait que
/// `SDWD` porterait des raretés `"3"`. Une commande de réparation ne peut donc
/// pas dépendre d'une comparaison de versions.
///
/// D'où le retour au découpage du Python : la mise à jour **se désactive**
/// quand il n'y a rien à faire, et la reconstruction vit ailleurs, toujours
/// disponible.
///
/// Un écart assumé : notre reconstruction **écrit** `last_update.txt`, que le
/// Python laissait en l'état. Chez lui, « Initialiser la base » sur une
/// installation sans fichier de version en laissait une sans version — donc
/// éternellement « à mettre à jour ». C'est le même trou que celui bouché le
/// 2026-09-04, une porte plus loin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Geste {
    /// Il n'y a pas de base — il faut la faire.
    Construire,
    /// Le serveur annonce autre chose que ce que l'on a.
    MettreAJour,
    /// Refaire la base telle quelle, pour la réparer. Toujours offerte.
    Reconstruire,
}

impl Geste {
    /// Le texte du bouton.
    fn bouton(self) -> &'static str {
        match self {
            Self::Construire => "⤓ Construire la base",
            Self::MettreAJour => "⟳ Mettre à jour la base",
            Self::Reconstruire => "⟳ Reconstruire la base",
        }
    }

    /// L'infobulle.
    fn explication(self) -> &'static str {
        match self {
            Self::Construire | Self::MettreAJour => {
                "Télécharge YGOJSON et YGOPRODeck, reconstruit cardinfo.db, \
                 puis enregistre la version obtenue."
            }
            Self::Reconstruire => {
                "Vous avez déjà la dernière version. À ne faire que si la base \
                 semble abîmée — elle sera refaite à l'identique."
            }
        }
    }

    /// Le titre de la fenêtre de confirmation.
    fn titre(self) -> &'static str {
        match self {
            Self::Construire => "Construire la base de référence ?",
            Self::MettreAJour => "Mettre à jour la base de référence ?",
            Self::Reconstruire => "Reconstruire la base à l'identique ?",
        }
    }

    /// Le bouton qui confirme.
    fn confirmation(self) -> &'static str {
        match self {
            Self::Construire => "Construire",
            Self::MettreAJour => "Mettre à jour",
            Self::Reconstruire => "Reconstruire quand même",
        }
    }
}

/// L'écran des options.
pub struct EcranOptions {
    config: Config,
    paths: Paths,

    langue: Langue,
    ui_langue: Langue,
    source: SourceImage,
    grille: (u8, u8),
    ordre: [CritereTri; 3],
    n_raretes: u8,
    completer_artworks: bool,
    font_scale: f64,

    /// Les priorités, éditées puis enregistrées d'un bloc.
    priorites: Vec<(String, i64)>,
    priorites_modifiees: bool,

    /// Ce que les changements de cette session impliquent.
    consequences: Consequences,
    /// Un retour a été demandé.
    retour: bool,
    /// Dernière erreur d'écriture, s'il y en a eu une.
    erreur: Option<String>,

    /// Ce que l'on sait de `bdd/cardinfo.db`, relu à l'ouverture.
    ///
    /// Lu une fois, pas à chaque image : c'est deux `stat` et un JSON, ce qui
    /// est peu — mais soixante fois par seconde, ce n'est plus peu.
    base: EtatLocal,
    /// Une mise à jour de la base tourne — le bouton doit le dire.
    maj_en_cours: bool,
    /// La confirmation ouverte, et pour quel geste.
    maj_a_confirmer: Option<Geste>,
    /// Une mise à jour a été demandée, et l'appelant ne l'a pas encore reprise.
    maj_demandee: bool,
    /// Le dernier verdict du contrôle de version, poussé par l'application.
    verdict: Option<Verdict>,
    /// Un re-contrôle de version a été demandé.
    verification_demandee: bool,

    /// Ce que la mise à jour des images vers Yugipedia ferait — calculé à la
    /// demande, jamais à chaque image : elle lit tous les classeurs.
    images_tirage: Option<Result<ygo_app::images_tirage::Installation, String>>,
    /// La mise à jour des images a été demandée, l'appelant ne l'a pas reprise.
    images_tirage_demandee: bool,
    /// Elle est partie : le bouton ne se reclique pas dans cette visite.
    images_tirage_lancee: bool,
}

impl std::fmt::Debug for EcranOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EcranOptions")
            .field("consequences", &self.consequences)
            .finish_non_exhaustive()
    }
}

impl EcranOptions {
    /// Lit les réglages courants.
    #[must_use]
    pub fn ouvrir(paths: Paths) -> Self {
        let config = Config::charger(paths.app_config());
        // Comme `_rarete_items_courants()` du Python : l'écran ne se contente
        // pas de lire le fichier, il le **répare** — absent, il le crée ; en
        // retard sur la base, il le complète. C'est ce qui rend l'installation
        // vierge utilisable sans que personne ait à saisir 43 raretés.
        let priorites_table = match ygo_app::raretes::synchroniser_priorites(&paths) {
            Ok((table, _)) => table,
            Err(e) => {
                tracing::warn!(erreur = %e, "priorités de rareté non synchronisées");
                Priorites::charger(paths.rarity_config())
            }
        };
        let mut priorites: Vec<(String, i64)> = priorites_table
            .iter()
            .map(|(nom, p)| (nom.to_owned(), p))
            .collect();
        // Par priorité croissante : c'est l'ordre du classeur, donc celui que
        // l'utilisateur a en tête.
        priorites.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));

        // `paths` part dans la structure : l'état de la base se lit avant.
        let paths_pour_base = paths.clone();

        Self {
            langue: config.langue(),
            ui_langue: config.ui_langue(),
            source: config.source_image(),
            grille: config.grille_defaut(),
            ordre: config.ordre_tri(),
            n_raretes: config.n_raretes_par_artwork(),
            completer_artworks: config.completer_artworks_yugipedia(),
            font_scale: config.font_scale(),
            priorites,
            priorites_modifiees: false,
            config,
            paths,
            consequences: Consequences::default(),
            retour: false,
            erreur: None,
            base: ygo_app::maj::etat_local(&paths_pour_base),
            maj_en_cours: false,
            maj_a_confirmer: None,
            maj_demandee: false,
            verdict: None,
            verification_demandee: false,
            images_tirage: None,
            images_tirage_demandee: false,
            images_tirage_lancee: false,
        }
    }

    /// Reprend la demande de mise à jour des images vers Yugipedia.
    ///
    /// Comme la mise à jour de la base, l'écran ne lance rien : la tâche
    /// écrit dans les classeurs puis télécharge, c'est une tâche de fond.
    pub fn images_yugipedia_demandee(&mut self) -> bool {
        std::mem::take(&mut self.images_tirage_demandee)
    }

    /// Analyse, sans rien écrire, ce que la mise à jour ferait.
    fn analyser_images_tirage(&mut self) {
        let reference = Priorites::charger(self.paths.rarity_config());
        self.images_tirage = Some(
            ygo_app::images_tirage::installation(&self.paths, &reference, false)
                .map_err(|e| e.to_string()),
        );
    }

    /// Reprend la demande de re-contrôle de version.
    pub fn verification_demandee(&mut self) -> bool {
        std::mem::take(&mut self.verification_demandee)
    }

    /// Reprend la demande de mise à jour de la base.
    ///
    /// L'écran ne lance rien lui-même : il n'a ni fil de travail ni client
    /// HTTP, et c'est très bien ainsi — la mise à jour est une tâche de fond
    /// comme les téléchargements, pas un effet de bord d'un écran de
    /// réglages.
    pub fn maj_demandee(&mut self) -> bool {
        std::mem::take(&mut self.maj_demandee)
    }

    /// Dit à l'écran où en est la mise à jour.
    ///
    /// La transition « en cours → terminée » relit l'état de la base : la
    /// version affichée doit être la nouvelle sans que l'utilisateur ait à
    /// ressortir de l'écran.
    pub fn signaler_maj(&mut self, en_cours: bool, verdict: Option<&Verdict>) {
        if self.maj_en_cours && !en_cours {
            self.base = ygo_app::maj::etat_local(&self.paths);
        }
        self.maj_en_cours = en_cours;
        self.verdict = verdict.cloned();
    }

    /// Reprend la demande de retour.
    pub fn retour_demande(&mut self) -> bool {
        std::mem::take(&mut self.retour)
    }

    /// Reprend les conséquences accumulées.
    ///
    /// L'appelant les consomme : c'est lui qui décide de rouvrir un classeur
    /// ou de recharger l'accueil.
    pub fn consequences(&mut self) -> Consequences {
        std::mem::take(&mut self.consequences)
    }

    /// Note une écriture ratée sans faire tomber l'écran.
    fn ecrire<T>(&mut self, quoi: &str, resultat: ygo_core::error::Result<T>) -> Option<T> {
        match resultat {
            Ok(v) => {
                self.erreur = None;
                Some(v)
            }
            Err(e) => {
                self.erreur = Some(format!("{quoi} : {e}"));
                None
            }
        }
    }

    /// Le bouton « Revenir au défaut » d'un réglage.
    ///
    /// # Pourquoi il n'apparaît que parfois
    ///
    /// Il n'est montré **que si le réglage a été décidé** — clé présente dans
    /// `app_config.json`. Un bouton offert en permanence pour ne rien faire
    /// dans la moitié des cas se clique une fois, ne produit rien, et n'est
    /// plus jamais cru.
    ///
    /// # Ce qu'il fait, et ce qu'il ne fait pas
    ///
    /// Il **efface** la clé, il n'y écrit pas la valeur par défaut. Un réglage
    /// qui ne décide pas doit être absent, pas égal : c'est la même règle
    /// qu'au niveau du classeur, où décocher « suivre les Options » retire la
    /// clé de `meta`. Sans quoi le réglage figerait le défaut d'aujourd'hui.
    fn retour_au_defaut<F>(&mut self, ui: &mut egui::Ui, cle: &str, oublier: F)
    where
        F: FnOnce(&Config) -> ygo_core::error::Result<()>,
    {
        if !self.config.est_defini(cle) {
            return;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let clique = ui
                .small_button("↺ Revenir au défaut")
                .on_hover_text("Retire ce réglage — l'application décidera de nouveau")
                .clicked();
            if clique
                && self
                    .ecrire("retour au défaut", oublier(&self.config))
                    .is_some()
            {
                self.grille = self.config.grille_defaut();
                self.ordre = self.config.ordre_tri();
                self.consequences.relire_accueil = true;
                self.consequences.relire_classeur = true;
            }
        });
    }
}

impl eframe::App for EcranOptions {
    fn ui(&mut self, racine: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.barre_du_haut(racine);
        self.confirmation_maj(racine.ctx());
        egui::CentralPanel::default().show(racine, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(8.0);
                self.affichage(ui);
                ui.add_space(16.0);
                self.tri(ui);
                ui.add_space(16.0);
                self.base_de_reference(ui);
                ui.add_space(16.0);
                self.donnees(ui);
                ui.add_space(16.0);
                self.maintenance(ui);
                ui.add_space(16.0);
                self.raretes(ui);
                ui.add_space(16.0);
            });
        });
    }
}

impl EcranOptions {
    fn barre_du_haut(&mut self, racine: &mut egui::Ui) {
        egui::Panel::top("entete-options").show(racine, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("◀ Mes classeurs").clicked() {
                    self.retour = true;
                }
                ui.add_space(8.0);
                ui.heading("Options");
                ui.add_space(12.0);
                ui.small(self.paths.app_config().display().to_string());
            });
            if let Some(erreur) = self.erreur.clone() {
                ui.colored_label(ui.visuals().error_fg_color, erreur);
            }
            ui.add_space(8.0);
        });
    }

    fn affichage(&mut self, ui: &mut egui::Ui) {
        section(ui, "Affichage", |ui| {
            ligne(ui, "Langue des cartes", |ui| {
                if choix_langue(ui, "langue-cartes", &mut self.langue) {
                    if let Some(v) = self.ecrire("langue", self.config.definir_langue(self.langue))
                    {
                        self.langue = v;
                        // La colonne de nom change : les cartes doivent être
                        // relues, pas seulement redessinées.
                        self.consequences.relire_classeur = true;
                    }
                }
            });

            ligne(ui, "Langue de l'interface", |ui| {
                // L'interface n'existe qu'en français (R7) : le choix est
                // gardé et enregistré, mais il le dit plutôt que de ne rien
                // faire en silence.
                ui.weak("français seulement pour l'instant").on_hover_text(
                    "L'interface en anglais n'est pas encore faite (R7). Le choix est enregistré.",
                );
                if choix_langue(ui, "langue-ui", &mut self.ui_langue) {
                    if let Some(v) = self.ecrire(
                        "langue de l'interface",
                        self.config.definir_ui_langue(self.ui_langue),
                    ) {
                        self.ui_langue = v;
                    }
                }
            });

            ligne(ui, "Source des images", |ui| {
                let mut source = self.source;
                egui::ComboBox::from_id_salt("source-images")
                    .selected_text(match source {
                        SourceImage::Ygoprodeck => "YGOPRODeck — une image par carte, la même pour toutes les raretés",
                        SourceImage::Yugipedia => "Yugipedia — l'image de chaque tirage : rareté et édition (repli YGOPRODeck)",
                    })
                    .width(460.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut source,
                            SourceImage::Ygoprodeck,
                            "YGOPRODeck — une image par carte, la même pour toutes les raretés",
                        );
                        ui.selectable_value(
                            &mut source,
                            SourceImage::Yugipedia,
                            "Yugipedia — l'image de chaque tirage : rareté et édition (repli YGOPRODeck)",
                        );
                    });
                if source != self.source {
                    self.source = source;
                    if let Some(v) = self.ecrire(
                        "source des images",
                        self.config.definir_source_image(source),
                    ) {
                        self.source = v;
                        // Les fichiers visés changent : cache périmé, et il
                        // manquera des images tant qu'elles ne sont pas
                        // téléchargées.
                        self.consequences.vider_images = true;
                        self.consequences.relire_classeur = true;
                    }
                }
            });

            ligne(ui, "Grille par défaut", |ui| {
                let (mut colonnes, mut lignes) = self.grille;
                let change = ui
                    .add(
                        egui::DragValue::new(&mut colonnes)
                            .range(GRILLE_MIN..=GRILLE_MAX)
                            .prefix("colonnes "),
                    )
                    .changed()
                    | ui.add(
                        egui::DragValue::new(&mut lignes)
                            .range(GRILLE_MIN..=GRILLE_MAX)
                            .prefix("lignes "),
                    )
                    .changed();
                if change {
                    if let Some(v) = self.ecrire(
                        "grille par défaut",
                        self.config.definir_grille_defaut(colonnes, lignes),
                    ) {
                        self.grille = v;
                        self.consequences.relire_accueil = true;
                    }
                }
                ui.small(format!(
                    "de {GRILLE_MIN} à {GRILLE_MAX} — s'applique aux classeurs qui n'ont pas la leur"
                ));
                self.retour_au_defaut(ui, Config::cle_grille(), |cfg| {
                    cfg.oublier_grille_defaut().map(|_| ())
                });
            });

            ligne(ui, "Taille du texte", |ui| {
                let mut echelle = self.font_scale;
                if ui
                    .add(
                        egui::Slider::new(&mut echelle, FONT_SCALE_MIN..=FONT_SCALE_MAX)
                            .fixed_decimals(2),
                    )
                    .drag_stopped()
                {
                    if let Some(v) =
                        self.ecrire("taille du texte", self.config.definir_font_scale(echelle))
                    {
                        self.font_scale = v;
                        // Tout de suite, sans redémarrage (R9).
                        crate::polices::appliquer_taille_texte(ui.ctx(), v);
                    }
                } else {
                    self.font_scale = echelle;
                }
            });
        });
    }

    fn tri(&mut self, ui: &mut egui::Ui) {
        section(ui, "Ordre de tri des cartes", |ui| {
            ui.small(
                "Les cartes sont comparées critère par critère, dans cet ordre. \
                 « Illustration » avant « Rareté » regroupe les tirages d'une même image.",
            );
            ui.add_space(8.0);

            let mut nouvel_ordre: Option<[CritereTri; 3]> = None;

            // Une seule façon de faire, et rien qui suggère le contraire.
            //
            // La première version mêlait une poignée de glissement ET deux
            // boutons « Monter / Descendre » sur la même ligne : la poignée dit
            // « attrape-moi », les boutons disent « clique-moi », et
            // l'utilisateur ne sait plus lequel des deux est le vrai geste.
            // Le glisser-déposer est celui du Python, et le plus direct pour
            // trois éléments — les boutons ont donc disparu.
            //
            // Ce qu'il en coûte : rien à l'écran ne dit qu'une ligne se
            // déplace tant qu'on n'a pas essayé. D'où la phrase d'aide, le
            // curseur main au survol (posé par `dnd_drag_source`) et la
            // poignée à gauche.
            ui.allocate_ui(egui::vec2(LARGEUR_LIGNE_TRI, 0.0), |ui| {
                for (rang, critere) in self.ordre.into_iter().enumerate() {
                    let id = egui::Id::new(("tri", critere.code()));
                    let (_, depose) = ui.dnd_drop_zone::<usize, ()>(
                        egui::Frame::new()
                            .inner_margin(egui::Margin::symmetric(6, 4))
                            .fill(ui.visuals().faint_bg_color)
                            .corner_radius(4),
                        |ui| {
                            ui.dnd_drag_source(id, rang, |ui| {
                                ui.horizontal(|ui| {
                                    poignee(ui);
                                    ui.add_space(8.0);
                                    ui.label(egui::RichText::new(format!("{}.", rang + 1)).weak());
                                    ui.add_space(4.0);
                                    ui.add(
                                        egui::Label::new(libelle_critere(critere))
                                            .selectable(false),
                                    );
                                });
                            });
                        },
                    );
                    if let Some(depuis) = depose {
                        nouvel_ordre = Some(deplacer(self.ordre, *depuis, rang));
                    }
                    ui.add_space(2.0);
                }
            });

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.small("Attrapez une ligne pour la déplacer.");
                self.retour_au_defaut(ui, Config::cle_ordre_tri(), |cfg| {
                    cfg.oublier_ordre_tri().map(|_| ())
                });
            });

            if let Some(ordre) = nouvel_ordre {
                if ordre != self.ordre {
                    if let Some(v) =
                        self.ecrire("ordre de tri", self.config.definir_ordre_tri(&ordre))
                    {
                        self.ordre = v;
                        self.consequences.relire_classeur = true;
                    }
                }
            }
        });
    }

    /// La base de référence — ce qu'on en sait, et le bouton qui la refait.
    ///
    /// # Pourquoi ce bloc existe
    ///
    /// `cardinfo.db` est la source de tout : les sets proposés à la création,
    /// les illustrations, les raretés. Jusqu'ici, la seule façon de la
    /// construire ou de la rafraîchir était `ygo-cli init` — alors que
    /// plusieurs messages de l'application, eux, disaient « lancez MAJ BDD ».
    /// Ils désignaient une commande que l'interface n'offrait pas.
    ///
    /// # Ce qui est montré, et pourquoi
    ///
    /// La version et la date, parce qu'un bouton « Mettre à jour » sans elles
    /// ne se décide pas : l'utilisateur ne saurait pas s'il a déjà la
    /// dernière. La taille, parce qu'une base à zéro octet est le symptôme
    /// d'une initialisation interrompue, et qu'on la voit d'un coup d'œil.
    fn base_de_reference(&mut self, ui: &mut egui::Ui) {
        section(ui, "Base de référence", |ui| {
            ligne(ui, "cardinfo.db", |ui| {
                if self.base.base_presente {
                    ui.label(self.base.resume());
                } else {
                    ui.colored_label(ui.visuals().warn_fg_color, self.base.resume());
                }
                ui.small(self.paths.cardinfo_db().display().to_string());
            });

            ligne(ui, "Version du serveur", |ui| match &self.verdict {
                Some(Verdict::AJour { version }) => {
                    ui.label(format!("À jour — le serveur annonce la {version}."));
                }
                Some(Verdict::AFaire { locale, distante }) => {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        match locale {
                            Some(v) => {
                                format!("⬆ Version {distante} disponible — vous avez la {v}.")
                            }
                            None => format!(
                                "⬆ Version {distante} disponible — aucune version \
                                 enregistrée ici."
                            ),
                        },
                    );
                }
                Some(Verdict::Indecidable { raison }) => {
                    ui.label("Serveur injoignable — impossible de comparer.");
                    ui.small(raison.clone());
                }
                None => {
                    ui.label("Contrôle en cours…");
                    ui.small("une requête à YGOPRODeck, faite au démarrage");
                }
            });

            ligne(ui, "", |ui| {
                // Le contrôle du démarrage échoue quand la machine n'était pas
                // encore en ligne. Sans ce bouton, il faudrait relancer
                // l'application pour le refaire.
                if ui
                    .small_button("↻ Revérifier")
                    .on_hover_text("Redemande la version au serveur")
                    .clicked()
                {
                    self.verification_demandee = true;
                }
            });

            let geste = self.geste();
            // L'intitulé suit la QUESTION posée, pas l'état de la réponse :
            // « Mise à jour » reste « Mise à jour » quand il n'y en a pas à
            // faire — c'est le bouton qui le dit, pas le titre de la ligne.
            let intitule = if geste == Geste::Construire {
                "Construction"
            } else {
                "Mise à jour"
            };
            ligne(ui, intitule, |ui| {
                // Rien à faire → bouton inerte, comme dans le Python. La
                // reconstruction forcée n'a pas disparu pour autant : elle est
                // en Maintenance, et l'infobulle y renvoie.
                let a_jour = geste == Geste::Reconstruire;
                let actif = !self.maj_en_cours && !a_jour;
                let reponse = ui
                    .add_enabled(
                        actif,
                        egui::Button::new(if a_jour {
                            "✔ La base est à jour"
                        } else {
                            geste.bouton()
                        }),
                    )
                    .on_hover_text(geste.explication());
                let reponse = if self.maj_en_cours {
                    reponse.on_disabled_hover_text("Une mise à jour est déjà en cours.")
                } else {
                    reponse.on_disabled_hover_text(
                        "Rien à mettre à jour. Pour refaire la base malgré tout : \
                         section Maintenance, ci-dessous.",
                    )
                };
                if reponse.clicked() {
                    self.maj_a_confirmer = Some(geste);
                }
                if self.maj_en_cours {
                    ui.spinner();
                    ui.small("en cours — l'avancement est en haut de la fenêtre");
                } else if !a_jour {
                    // Mesuré le 2026-09-05 sur le poste de l'utilisateur :
                    // archive de 33 Mo, 10 s de bout en bout. Le « 170 Mo »
                    // annoncé jusque-là était la taille de la BASE, pas du
                    // téléchargement — un chiffre cinq fois trop gros, qui
                    // faisait hésiter devant une opération de dix secondes.
                    ui.small("environ 35 Mo à télécharger — moins d'une minute");
                }
            });
        });
    }

    /// Ce que la ligne « Mise à jour » propose, d'après ce que l'on sait.
    ///
    /// [`Geste::Reconstruire`] y signifie « rien à mettre à jour » : la ligne
    /// se désactive et renvoie vers la Maintenance, qui elle l'offre toujours.
    fn geste(&self) -> Geste {
        if !self.base.base_presente {
            Geste::Construire
        } else if matches!(self.verdict, Some(Verdict::AJour { .. })) {
            Geste::Reconstruire
        } else {
            Geste::MettreAJour
        }
    }

    /// La section Maintenance — les opérations de réparation.
    ///
    /// Portage de `ecran_options.py` §Maintenance, qui en offre six. Une seule
    /// est portée à ce jour ; les cinq autres existent en `ygo-cli` (`raretes`,
    /// `doublons`, `overframe`) ou n'ont pas d'équivalent.
    /// Passer les classeurs existants à l'image Yugipedia de chaque tirage.
    ///
    /// Les classeurs créés depuis le 2026-09-30 la reçoivent à la création.
    /// Ceux d'avant gardent l'image YGOPRODeck de l'illustration — la même
    /// pour toutes les raretés — tant qu'on ne la leur pose pas : c'est ce que
    /// fait ce bouton, l'équivalent de `ygo-cli images-tirage --corriger`.
    ///
    /// Analyse d'abord, sur clic : l'utilisateur voit combien de lignes et
    /// d'images sont en jeu, et combien de temps le téléchargement prendra au
    /// rythme imposé par Yugipedia, avant de décider.
    fn images_des_classeurs(&mut self, ui: &mut egui::Ui) {
        if self.source != SourceImage::Yugipedia {
            ui.weak("choisissez Yugipedia dans « Source des images » pour en profiter");
            return;
        }
        if ui
            .button("🔍 Analyser")
            .on_hover_text(
                "Cherche, classeur par classeur, les lignes qui peuvent recevoir \
                 l'image de leur tirage. N'écrit rien.",
            )
            .clicked()
        {
            self.analyser_images_tirage();
            self.images_tirage_lancee = false;
        }
        match &self.images_tirage {
            None => {
                ui.small(
                    "l'image de chaque tirage — rareté et édition — dans les classeurs existants",
                );
            }
            Some(Err(e)) => {
                ui.colored_label(ui.visuals().error_fg_color, e.clone());
            }
            Some(Ok(vue)) if vue.a_poser == 0 => {
                ui.small(format!(
                    "{} classeur(s) : déjà à jour ({} ligne(s) avec l'image de leur tirage)",
                    vue.classeurs, vue.deja
                ));
            }
            Some(Ok(vue)) => {
                let resume = format!(
                    "{} ligne(s) dans {} classeur(s) · ≈ {} image(s) à télécharger, {}",
                    vue.a_poser,
                    vue.touches.len(),
                    vue.a_telecharger,
                    duree(vue.a_telecharger)
                );
                let clique = ui
                    .add_enabled(
                        !self.images_tirage_lancee,
                        egui::Button::new("⟳ Mettre à jour vers Yugipedia"),
                    )
                    .on_hover_text(
                        "Seule l'adresse de l'image change : ni possession, ni quantité, \
                         ni état. Les images se téléchargent ensuite en arrière-plan, \
                         une par seconde comme Yugipedia le demande.",
                    )
                    .on_disabled_hover_text("Lancée — suivez la bande de progression.")
                    .clicked();
                ui.small(resume);
                if clique {
                    self.images_tirage_demandee = true;
                    self.images_tirage_lancee = true;
                }
            }
        }
    }

    fn maintenance(&mut self, ui: &mut egui::Ui) {
        section(ui, "Maintenance", |ui| {
            ui.small(
                "À utiliser en cas de problème — ces opérations ne dépendent \
                 d'aucun contrôle de version.",
            );
            ui.add_space(6.0);
            ligne(ui, "Base de référence", |ui| {
                let reponse = ui
                    .add_enabled(
                        !self.maj_en_cours,
                        egui::Button::new(Geste::Reconstruire.bouton()),
                    )
                    .on_hover_text(Geste::Reconstruire.explication())
                    .on_disabled_hover_text("Une mise à jour est déjà en cours.");
                if reponse.clicked() {
                    self.maj_a_confirmer = Some(Geste::Reconstruire);
                }
                ui.small(
                    "refait cardinfo.db à l'identique — cartes manquantes, raretés aberrantes",
                );
            });
        });
    }

    /// La confirmation de mise à jour.
    ///
    /// Elle existe parce que l'opération remplace la base. Elle dit surtout ce
    /// qui n'est **pas** touché — les classeurs et les possessions vivent dans
    /// d'autres fichiers — parce que c'est la question que l'on se pose devant
    /// un bouton qui parle de reconstruire une base.
    ///
    /// Et quand il n'y a rien à mettre à jour, elle le dit **avant** de faire
    /// descendre l'archive : cf. [`Geste`].
    fn confirmation_maj(&mut self, ctx: &egui::Context) {
        let Some(geste) = self.maj_a_confirmer else {
            return;
        };
        let mut ouverte = true;
        let mut confirme = false;
        egui::Window::new(geste.titre())
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .open(&mut ouverte)
            .show(ctx, |ui| {
                ui.set_min_width(460.0);
                ui.add_space(4.0);
                if geste == Geste::Reconstruire {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        match &self.verdict {
                            Some(Verdict::AJour { version }) => format!(
                                "Vous avez déjà la dernière version ({version}) — la \
                                 base sera refaite à l'identique."
                            ),
                            _ => "La base sera entièrement refaite depuis les sources.".to_owned(),
                        },
                    );
                    ui.small(
                        "Utile si elle semble abîmée : cartes manquantes, raretés \
                         aberrantes. Sinon, rien ne changera.",
                    );
                    ui.add_space(6.0);
                }
                ui.label(
                    "L'archive YGOJSON sera téléchargée — environ 35 Mo — puis \
                     cardinfo.db entièrement reconstruite (≈ 160 Mo sur disque).",
                );
                ui.add_space(6.0);
                ui.label("Vos classeurs et vos possessions ne sont pas touchés :");
                ui.small("ils vivent dans classeur/*.db, que cette opération ne lit même pas.");
                ui.add_space(6.0);
                ui.small(
                    "La base est construite à côté puis mise en place d'un coup : \
                     une coupure en cours de route laisse l'ancienne intacte.",
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button(geste.confirmation()).clicked() {
                        confirme = true;
                    }
                    if ui.button("Annuler").clicked() {
                        self.maj_a_confirmer = None;
                    }
                });
                ui.add_space(4.0);
            });
        if !ouverte {
            self.maj_a_confirmer = None;
        }
        if confirme {
            self.maj_a_confirmer = None;
            self.maj_demandee = true;
            // Levé ici et pas à l'arrivée du premier événement : le fil met un
            // instant à répondre, et un bouton resté actif se reclique.
            self.maj_en_cours = true;
        }
    }

    fn donnees(&mut self, ui: &mut egui::Ui) {
        section(ui, "Données", |ui| {
            ligne(ui, "Raretés par artwork", |ui| {
                let mut n = self.n_raretes;
                if ui
                    .add(
                        egui::DragValue::new(&mut n)
                            .range(N_RARETES_MIN..=N_RARETES_MAX)
                            .speed(0.2),
                    )
                    .changed()
                {
                    if let Some(v) = self.ecrire(
                        "raretés par artwork",
                        self.config.definir_n_raretes_par_artwork(n),
                    ) {
                        self.n_raretes = v;
                        self.consequences.relire_classeur = true;
                    }
                }
                ui.small(if self.n_raretes == 0 {
                    "0 — toutes les raretés sont affichées".to_owned()
                } else {
                    format!(
                        "{} rareté(s) par illustration, les plus rares gardées",
                        self.n_raretes
                    )
                });
            });

            ligne(ui, "Images des classeurs", |ui| {
                self.images_des_classeurs(ui)
            });

            ligne(ui, "Artworks Yugipedia", |ui| {
                let mut actif = self.completer_artworks;
                if ui
                    .checkbox(&mut actif, "compléter les variantes à la création")
                    .changed()
                {
                    if let Some(v) = self.ecrire(
                        "artworks Yugipedia",
                        self.config.definir_completer_artworks_yugipedia(actif),
                    ) {
                        self.completer_artworks = v;
                    }
                }
            });
        });
    }

    fn raretes(&mut self, ui: &mut egui::Ui) {
        section(ui, "Priorités de rareté", |ui| {
            ui.small(
                "Plus le nombre est élevé, plus la rareté est considérée comme rare. \
                 C'est l'entrée du tri et du filtre « N raretés par artwork ».",
            );
            ui.add_space(6.0);

            if self.priorites.is_empty() {
                ui.label("rarity_config.json est vide ou illisible.");
                return;
            }

            egui::ScrollArea::vertical()
                .id_salt("liste-priorites")
                .max_height(260.0)
                .show(ui, |ui| {
                    for (nom, priorite) in &mut self.priorites {
                        ui.horizontal(|ui| {
                            ui.add_sized([260.0, 20.0], egui::Label::new(nom.as_str()).truncate());
                            if ui
                                .add(egui::DragValue::new(priorite).range(0..=999).speed(0.2))
                                .changed()
                            {
                                self.priorites_modifiees = true;
                            }
                        });
                    }
                });

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(self.priorites_modifiees, egui::Button::new("Enregistrer"))
                    .clicked()
                {
                    let table = Priorites::depuis_paires(
                        self.priorites
                            .iter()
                            .map(|(nom, p)| (nom.clone(), *p))
                            .collect::<Vec<_>>(),
                    );
                    let resultat = table.enregistrer(self.paths.rarity_config());
                    if self.ecrire("priorités de rareté", resultat).is_some() {
                        self.priorites_modifiees = false;
                        self.consequences.relire_classeur = true;
                    }
                }
                if self.priorites_modifiees {
                    ui.small("modifications non enregistrées");
                }
            });
        });
    }
}

/// Largeur de la zone des critères de tri.
///
/// Bornée à dessein : une ligne large de tout l'écran rejette ses commandes à
/// un mètre de leur libellé, ce qui était le défaut de la première version.
const LARGEUR_LIGNE_TRI: f32 = 460.0;

/// Déplace le critère de rang `de` vers le rang `vers`.
///
/// C'est **l'insertion** et non l'échange : glisser le troisième critère en
/// tête doit décaler les deux autres d'un rang, pas permuter le premier et le
/// troisième. Un glisser-déposer qui permute donne un résultat que personne
/// n'attend dès qu'on saute une position.
///
/// Un rang hors bornes rend l'ordre inchangé — un lâcher hors zone ne doit
/// rien casser.
///
/// ```
/// use ygo_core::config::CritereTri::{Artwork, Numero, Rarete};
/// use ygo_ui::options::deplacer;
///
/// // Le troisième passe en tête : les deux autres reculent d'un rang.
/// assert_eq!(
///     deplacer([Numero, Artwork, Rarete], 2, 0),
///     [Rarete, Numero, Artwork]
/// );
/// ```
#[must_use]
pub fn deplacer(ordre: [CritereTri; 3], de: usize, vers: usize) -> [CritereTri; 3] {
    if de >= 3 || vers >= 3 || de == vers {
        return ordre;
    }
    let mut liste: Vec<CritereTri> = ordre.to_vec();
    let critere = liste.remove(de);
    liste.insert(vers, critere);
    [
        liste.first().copied().unwrap_or(CritereTri::Numero),
        liste.get(1).copied().unwrap_or(CritereTri::Rarete),
        liste.get(2).copied().unwrap_or(CritereTri::Artwork),
    ]
}

/// La poignée de déplacement — six points, dessinés.
///
/// Dessinée et non écrite : les glyphes qui ressemblent à une poignée (`⣿`,
/// `⋮⋮`, `≡`) ne sont dans aucune police embarquée par egui et rendent des
/// carrés vides. C'est le même défaut qui a fait disparaître les flèches `▲` et
/// `▼` de la première version — autant ne plus dépendre d'une police pour un
/// signe de deux millimètres.
/// La poignée de glissement d'une ligne réordonnable.
///
/// Publique parce que l'ordre de tri se règle à deux endroits — ici pour
/// toute l'installation, et dans les réglages d'un classeur pour lui seul. Le
/// **geste** doit être le même aux deux : une poignée qu'on attrape.
pub fn poignee(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 18.0), egui::Sense::hover());
    let couleur = ui.visuals().weak_text_color();
    let peintre = ui.painter();
    for ligne in 0..3 {
        for colonne in 0..2 {
            #[allow(clippy::cast_precision_loss)]
            let centre = egui::pos2(
                rect.left() + 3.0 + colonne as f32 * 5.0,
                rect.top() + 4.0 + ligne as f32 * 5.0,
            );
            peintre.circle_filled(centre, 1.2, couleur);
        }
    }
}

fn section(ui: &mut egui::Ui, titre: &str, contenu: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::group(ui.style())
        .corner_radius(8.0)
        .inner_margin(12.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width() - 24.0);
            ui.label(egui::RichText::new(titre).strong());
            ui.add_space(8.0);
            contenu(ui);
        });
}

/// Une durée de téléchargement, au rythme d'une image par seconde environ.
///
/// Arrondie à ce qui aide à décider : « quelques secondes », des minutes, des
/// heures — pas « 3 742 s ».
fn duree(images: usize) -> String {
    // 1,1 s par image : le limiteur Yugipedia (cf. `ygo_sources::http`).
    let secondes = images.saturating_mul(11) / 10;
    match secondes {
        0..=59 => "moins d'une minute".to_owned(),
        60..=3599 => format!("≈ {} min", secondes.div_ceil(60)),
        _ => format!("≈ {} h {:02}", secondes / 3600, (secondes % 3600) / 60),
    }
}

fn ligne(ui: &mut egui::Ui, intitule: &str, contenu: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.add_sized([200.0, 20.0], egui::Label::new(intitule));
        contenu(ui);
    });
    ui.add_space(6.0);
}

fn choix_langue(ui: &mut egui::Ui, id: &str, langue: &mut Langue) -> bool {
    let avant = *langue;
    egui::ComboBox::from_id_salt(id)
        .selected_text(match *langue {
            Langue::Fr => "Français",
            Langue::En => "English",
        })
        .width(160.0)
        .show_ui(ui, |ui| {
            ui.selectable_value(langue, Langue::Fr, "Français");
            ui.selectable_value(langue, Langue::En, "English");
        });
    *langue != avant
}

/// Le nom d'un critère de tri, tel qu'il s'affiche.
#[must_use]
pub fn libelle_critere(critere: CritereTri) -> &'static str {
    match critere {
        CritereTri::Numero => "Numéro de collection",
        CritereTri::Rarete => "Rareté",
        CritereTri::Artwork => "Illustration",
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    #[test]
    fn la_duree_se_dit_a_l_echelle_qui_aide_a_decider() {
        assert_eq!(duree(0), "moins d'une minute");
        assert_eq!(duree(50), "moins d'une minute");
        assert_eq!(duree(100), "≈ 2 min");
        assert_eq!(duree(3000), "≈ 55 min");
        assert_eq!(duree(4000), "≈ 1 h 13");
    }

    #[test]
    fn les_consequences_se_fusionnent_sans_rien_perdre() {
        let a = Consequences {
            relire_classeur: true,
            ..Consequences::default()
        };
        let b = Consequences {
            vider_images: true,
            ..Consequences::default()
        };
        let fusion = a.avec(b);
        assert!(fusion.relire_classeur && fusion.vider_images);
        assert!(!fusion.relire_accueil);
        assert!(fusion.quelque_chose());
    }

    #[test]
    fn sans_changement_il_n_y_a_rien_a_faire() {
        assert!(!Consequences::default().quelque_chose());
    }

    /// Les conséquences se **consomment** : l'application ne doit pas rouvrir
    /// le classeur à chaque image parce qu'un réglage a changé une fois.
    #[test]
    fn les_consequences_se_consomment() {
        let tmp = tempfile::tempdir().unwrap();
        let mut ecran = EcranOptions::ouvrir(Paths::depuis_racine(tmp.path()));
        ecran.consequences.relire_classeur = true;

        assert!(ecran.consequences().relire_classeur);
        assert!(!ecran.consequences().quelque_chose(), "une seule fois");
    }

    /// Chaque critère de tri a un libellé qui lui est propre.
    /// Le déplacement est une **insertion**, pas un échange : glisser le
    /// troisième en tête décale les deux autres, il ne permute pas.
    #[test]
    fn deplacer_insere_au_lieu_de_permuter() {
        use CritereTri::{Artwork, Numero, Rarete};
        let depart = [Numero, Artwork, Rarete];

        assert_eq!(deplacer(depart, 2, 0), [Rarete, Numero, Artwork]);
        assert_eq!(deplacer(depart, 0, 2), [Artwork, Rarete, Numero]);

        // Un échange donnerait [Rarete, Artwork, Numero] pour le premier cas :
        // ce n'est pas ce que l'utilisateur voit en déplaçant une ligne.
        assert_ne!(deplacer(depart, 2, 0), [Rarete, Artwork, Numero]);
    }

    /// Voisins : le déplacement d'un cran est une permutation, et c'est ce que
    /// font les boutons « Monter » et « Descendre ».
    #[test]
    fn deplacer_d_un_cran_permute_les_voisins() {
        use CritereTri::{Artwork, Numero, Rarete};
        let depart = [Numero, Artwork, Rarete];
        assert_eq!(deplacer(depart, 1, 0), [Artwork, Numero, Rarete]);
        assert_eq!(deplacer(depart, 1, 2), [Numero, Rarete, Artwork]);
    }

    /// Un lâcher hors zone, ou sur soi-même, ne doit rien casser.
    #[test]
    fn un_deplacement_impossible_laisse_l_ordre_intact() {
        use CritereTri::{Artwork, Numero, Rarete};
        let depart = [Numero, Artwork, Rarete];
        assert_eq!(deplacer(depart, 1, 1), depart, "sur soi-même");
        assert_eq!(deplacer(depart, 9, 0), depart, "source hors bornes");
        assert_eq!(deplacer(depart, 0, 9), depart, "cible hors bornes");
    }

    /// Quel que soit le déplacement, les trois critères restent présents une
    /// fois chacun — un ordre partiel ne peut pas naître d'un glisser-déposer.
    #[test]
    fn aucun_deplacement_ne_perd_ni_ne_duplique_un_critere() {
        use CritereTri::{Artwork, Numero, Rarete};
        let depart = [Numero, Artwork, Rarete];
        for de in 0..3 {
            for vers in 0..3 {
                let apres = deplacer(depart, de, vers);
                for critere in [Numero, Artwork, Rarete] {
                    assert_eq!(
                        apres.iter().filter(|c| **c == critere).count(),
                        1,
                        "{critere:?} après {de} → {vers}"
                    );
                }
            }
        }
    }

    #[test]
    fn les_trois_criteres_ont_des_libelles_distincts() {
        let libelles = [
            libelle_critere(CritereTri::Numero),
            libelle_critere(CritereTri::Rarete),
            libelle_critere(CritereTri::Artwork),
        ];
        let mut tries = libelles;
        tries.sort_unstable();
        let avant = tries.len();
        let mut sans_doublon = tries.to_vec();
        sans_doublon.dedup();
        assert_eq!(sans_doublon.len(), avant);
    }

    /// Une installation vide ne fait pas tomber l'écran : il s'ouvre sur les
    /// défauts, et la liste des priorités est simplement vide.
    #[test]
    fn une_installation_vide_ouvre_l_ecran_sur_les_defauts() {
        let tmp = tempfile::tempdir().unwrap();
        let paths_du_test = Paths::depuis_racine(tmp.path());
        let ecran = EcranOptions::ouvrir(paths_du_test.clone());
        assert_eq!(ecran.langue, Langue::Fr);
        assert_eq!(ecran.source, SourceImage::Yugipedia);
        assert_eq!(ecran.grille, (3, 3));
        assert_eq!(ecran.n_raretes, 0);
        assert!(ecran.completer_artworks);
        assert!(!ecran.base.base_presente, "et la base manque, forcément");
        // Longtemps ce test affirmait `priorites.is_empty()` — il figeait le
        // défaut du 2026-09-05. Une installation vierge reçoit désormais les
        // raretés de repli, sans quoi le tri par rareté ne rangerait rien.
        assert_eq!(
            ecran.priorites.len(),
            ygo_core::rarity::RARETES_REPLI.len(),
            "l'écran répare le fichier absent au lieu de s'ouvrir sans priorités"
        );
        assert!(paths_du_test.rarity_config().is_file(), "et il l'écrit");
    }

    /// Le bouton de mise à jour ne demande rien tant qu'on ne l'a pas
    /// confirmé, et sa demande se reprend une seule fois.
    #[test]
    fn la_demande_de_mise_a_jour_se_consomme() {
        let tmp = tempfile::tempdir().unwrap();
        let mut ecran = EcranOptions::ouvrir(Paths::depuis_racine(tmp.path()));
        assert!(!ecran.maj_demandee(), "rien n'est demandé à l'ouverture");

        ecran.maj_demandee = true;
        assert!(ecran.maj_demandee());
        assert!(!ecran.maj_demandee(), "et pas deux fois");
    }

    /// La fin d'une mise à jour relit la base : la version affichée doit être
    /// la nouvelle sans qu'on ait à ressortir de l'écran.
    #[test]
    fn la_fin_de_la_mise_a_jour_relit_la_base() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(tmp.path());
        let mut ecran = EcranOptions::ouvrir(paths.clone());
        assert!(!ecran.base.base_presente);

        ecran.signaler_maj(true, None);
        // La base apparaît pendant que l'écran croit la mise à jour en cours.
        paths.creer_dossiers().unwrap();
        std::fs::write(paths.cardinfo_db(), b"une base, desormais").unwrap();
        assert!(
            !ecran.base.base_presente,
            "rien n'est relu tant que ça tourne"
        );

        ecran.signaler_maj(false, None);
        assert!(ecran.base.base_presente, "la transition relit");
    }

    /// Les priorités sont montrées par ordre croissant — l'ordre du classeur,
    /// donc celui que l'utilisateur a en tête.
    #[test]
    fn les_priorites_sont_montrees_de_la_moins_rare_a_la_plus_rare() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(tmp.path());
        std::fs::create_dir_all(paths.rarity_config().parent().unwrap()).unwrap();
        Priorites::depuis_paires([("Secret Rare", 5), ("Common", 1), ("Ultra Rare", 4)])
            .enregistrer(paths.rarity_config())
            .unwrap();

        let ecran = EcranOptions::ouvrir(paths);
        let noms: Vec<&str> = ecran.priorites.iter().map(|(n, _)| n.as_str()).collect();

        // Les trois rangs choisis, dans leur ordre, en tête.
        assert_eq!(
            noms.get(..3),
            Some(["Common", "Ultra Rare", "Secret Rare"].as_slice())
        );

        // Et les raretés que le fichier ignorait, ajoutées **derrière** — la
        // règle du Python : on complète, on ne renumérote jamais. « Rare »
        // atterrit donc après « Secret Rare », ce qui surprend mais respecte
        // le classement que l'utilisateur avait posé.
        assert!(noms.len() > 3, "les manquantes ont été ajoutées : {noms:?}");
        let rang = |cible: &str| noms.iter().position(|n| *n == cible);
        assert!(rang("Rare") > rang("Secret Rare"));
        assert!(
            ecran.priorites.iter().map(|(_, p)| *p).is_sorted(),
            "et l'ordre affiché reste croissant : {:?}",
            ecran.priorites
        );
    }

    /// Le découpage du Python, rétabli le 2026-09-05 : la ligne « Mise à jour »
    /// se désactive quand il n'y a rien à faire — [`Geste::Reconstruire`] y
    /// signifie « rien à mettre à jour » — et la reconstruction forcée vit en
    /// Maintenance, hors de tout verdict.
    #[test]
    fn le_bouton_dit_ce_qu_il_fait_vraiment() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(tmp.path());
        let mut ecran = EcranOptions::ouvrir(paths.clone());

        // Pas de base : il n'y a rien à mettre à jour, il y a à construire.
        assert_eq!(ecran.geste(), Geste::Construire);

        paths.creer_dossiers().unwrap();
        std::fs::write(paths.cardinfo_db(), b"une base").unwrap();
        ecran.signaler_maj(true, None);
        ecran.signaler_maj(false, None);

        // Base présente, verdict inconnu : « mettre à jour » reste le pari
        // honnête — on ne sait pas encore.
        assert_eq!(ecran.geste(), Geste::MettreAJour);

        ecran.signaler_maj(
            false,
            Some(&Verdict::AFaire {
                locale: Some("146.68".to_owned()),
                distante: "147.01".to_owned(),
            }),
        );
        assert_eq!(ecran.geste(), Geste::MettreAJour);

        ecran.signaler_maj(
            false,
            Some(&Verdict::AJour {
                version: "146.68".to_owned(),
            }),
        );
        assert_eq!(
            ecran.geste(),
            Geste::Reconstruire,
            "à jour : le bouton ne peut plus prétendre mettre à jour"
        );
    }

    /// Une base absente l'emporte sur tout verdict : même « à jour », il n'y a
    /// rien à reconstruire, il y a à construire.
    #[test]
    fn une_base_absente_l_emporte_sur_le_verdict() {
        let tmp = tempfile::tempdir().unwrap();
        let mut ecran = EcranOptions::ouvrir(Paths::depuis_racine(tmp.path()));
        ecran.signaler_maj(
            false,
            Some(&Verdict::AJour {
                version: "146.68".to_owned(),
            }),
        );
        assert_eq!(ecran.geste(), Geste::Construire);
    }

    /// Chaque geste porte ses propres mots — sans quoi la distinction ne
    /// servirait à rien.
    #[test]
    fn les_trois_gestes_ont_des_mots_distincts() {
        let gestes = [Geste::Construire, Geste::MettreAJour, Geste::Reconstruire];
        for champ in [Geste::bouton, Geste::titre, Geste::confirmation] {
            let mut vus = std::collections::HashSet::new();
            for geste in gestes {
                let mot = champ(geste);
                assert!(!mot.is_empty());
                assert!(vus.insert(mot), "mot répété entre gestes : {mot}");
            }
        }
    }

    /// La reconstruction ne dépend d'aucun verdict : c'est tout l'objet de la
    /// section Maintenance, et ce que le Python offrait déjà.
    #[test]
    fn la_reconstruction_est_offerte_quel_que_soit_le_verdict() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(tmp.path());
        paths.creer_dossiers().unwrap();
        std::fs::write(paths.cardinfo_db(), b"une base").unwrap();
        let mut ecran = EcranOptions::ouvrir(paths);

        for verdict in [
            None,
            Some(Verdict::AJour {
                version: "146.68".to_owned(),
            }),
            Some(Verdict::AFaire {
                locale: None,
                distante: "147.01".to_owned(),
            }),
            Some(Verdict::Indecidable {
                raison: "hors ligne".to_owned(),
            }),
        ] {
            ecran.signaler_maj(false, verdict.as_ref());
            // Le bouton de Maintenance ne consulte pas `geste()` : il pose
            // toujours le même. On éprouve ici que rien ne l'en empêche.
            ecran.maj_a_confirmer = Some(Geste::Reconstruire);
            assert_eq!(ecran.maj_a_confirmer, Some(Geste::Reconstruire));
            ecran.maj_a_confirmer = None;
        }
    }

    /// La confirmation porte le geste cliqué, pas celui que l'état suggère :
    /// reconstruire depuis la Maintenance ne doit pas afficher la fenêtre de
    /// mise à jour parce que le serveur annonce une version neuve.
    #[test]
    fn la_confirmation_porte_le_geste_clique() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::depuis_racine(tmp.path());
        paths.creer_dossiers().unwrap();
        std::fs::write(paths.cardinfo_db(), b"une base").unwrap();
        let mut ecran = EcranOptions::ouvrir(paths);
        ecran.signaler_maj(
            false,
            Some(&Verdict::AFaire {
                locale: None,
                distante: "147.01".to_owned(),
            }),
        );
        assert_eq!(ecran.geste(), Geste::MettreAJour);

        ecran.maj_a_confirmer = Some(Geste::Reconstruire);
        assert_eq!(
            ecran.maj_a_confirmer,
            Some(Geste::Reconstruire),
            "le clic l'emporte sur ce que l'état aurait proposé"
        );
    }
}
