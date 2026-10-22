// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! La fiche d'une carte, ouverte au clic.
//!
//! # Pourquoi le clic, et pas le survol
//!
//! Dans le classeur, le survol est déjà pris : il lève l'atténuation d'une
//! carte non possédée **et** arme la molette, qui ajoute ou retire un
//! exemplaire. C'est la commande la plus rapide pour saisir une collection —
//! on balaie la page sans jamais viser un bouton. Une fenêtre qui s'ouvrirait
//! au survol se dresserait entre l'utilisateur et la carte suivante pendant
//! ce balayage.
//!
//! Le clic gauche, lui, était libre : la case n'a `Sense::click()` que parce
//! que le menu contextuel l'exige. Et c'est déjà ce que faisait le Python,
//! dont `dialog_carte.py` s'ouvre au clic sur la carte de la grille.
//!
//! # Pourquoi une surimpression, et pas un panneau latéral
//!
//! Les 560 points laissés libres sur les côtés d'un écran 16:9 donnent envie
//! d'y loger la fiche. Mais depuis que [`ygo_app::classeur::taille_carte`] se
//! nourrit de `available_width` **et** `available_height`, un `SidePanel`
//! amputerait la largeur disponible : les cartes rétréciraient à l'ouverture
//! et regrandiraient à la fermeture. Tout le classeur se réagencerait à
//! chaque clic. Une fenêtre par-dessus ne touche à rien.

use eframe::egui;
use ygo_app::cardmarket::{euros, Cible, Offre, PrixOffre};
use ygo_app::fiche::{Fiche, Resolution};

use crate::images::{Cache, Cle};

/// Largeur de l'illustration dans la fiche, en points.
///
/// Les images du cache font **813 × 1 185 pixels** (1 872 des 1 878 fichiers
/// mesurés sur l'installation réelle). À 420 points on reste largement sous
/// cette définition — donc pas de flou — et le pavé de texte imprimé
/// redevient déchiffrable, là où les 310 points d'une case du classeur n'en
/// donnent qu'une bouillie grise.
pub const LARGEUR_IMAGE: f32 = 420.0;

/// Hauteur de l'illustration — celle d'une carte, 59 × 86 mm — et donc de
/// la colonne de texte à côté.
pub const HAUTEUR_IMAGE: f32 = LARGEUR_IMAGE * 86.0 / 59.0;

/// Largeur de la colonne de texte, à côté de l'illustration.
pub const LARGEUR_TEXTE: f32 = 460.0;

/// Ce que la fiche demande à l'écran qui la contient.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Demande {
    /// Rien.
    #[default]
    Rien,
    /// Fermer la fiche.
    Fermer,
    /// Montrer la carte précédente de la page.
    Precedente,
    /// Montrer la suivante.
    Suivante,
}

/// L'état de la fiche ouverte.
#[derive(Debug, Default)]
pub struct EtatFiche {
    /// La carte montrée, ou rien si la fiche est fermée.
    fiche: Option<Fiche>,
    /// La langue choisie à la main ; sinon la préférence de l'écran.
    langue: Option<String>,
}

impl EtatFiche {
    /// Ouvre la fiche sur cette carte.
    ///
    /// # La langue choisie survit d'une carte à l'autre
    ///
    /// La première version l'oubliait à chaque ouverture. Une mutation qui
    /// retirait cet oubli survivait à tous les tests — parce que
    /// [`Self::langue_retenue`] écarte déjà une langue que la carte n'offre
    /// pas. L'oubli ne protégeait donc de rien ; il ne faisait que coûter un
    /// clic.
    ///
    /// Et le coût n'est pas anecdotique : feuilleter un classeur japonais,
    /// c'est ouvrir une carte, passer en japonais, fermer, ouvrir la
    /// suivante. Avec l'oubli, il fallait rebasculer à chaque fois.
    pub fn ouvrir(&mut self, fiche: Fiche) {
        self.fiche = Some(fiche);
    }

    /// Ferme la fiche.
    pub fn fermer(&mut self) {
        self.fiche = None;
        self.langue = None;
    }

    /// La fiche est-elle ouverte ?
    #[must_use]
    pub fn est_ouverte(&self) -> bool {
        self.fiche.is_some()
    }

    /// La carte montrée.
    #[must_use]
    pub fn carte(&self) -> Option<&Fiche> {
        self.fiche.as_ref()
    }

    /// La langue effectivement retenue : le choix de l'utilisateur s'il est
    /// encore offert par cette carte, sinon la préférence de l'écran.
    ///
    /// La vérification n'est pas superflue : en passant d'une carte
    /// occidentale lue en `de` à une carte japonaise qui n'a que `en` et
    /// `ja`, un choix conservé sans contrôle laisserait la fiche vide.
    #[must_use]
    pub fn langue_retenue(&self, francais: bool, cjk: bool) -> String {
        let preferee = if francais { "fr" } else { "en" };
        let Some(f) = &self.fiche else {
            return preferee.to_owned();
        };
        let offertes = langues_affichables(f, preferee, cjk);
        if let Some(choisie) = &self.langue {
            if offertes.iter().any(|l| l == choisie) {
                return choisie.clone();
            }
        }
        offertes
            .first()
            .cloned()
            .unwrap_or_else(|| preferee.to_owned())
    }

    /// Dessine la fiche, et rend ce que l'utilisateur demande.
    pub fn afficher(
        &mut self,
        ctx: &egui::Context,
        cache: &mut Cache,
        dossier_images: &std::path::Path,
        francais: bool,
    ) -> Demande {
        let Some(fiche) = self.fiche.clone() else {
            return Demande::Rien;
        };
        let mut demande = Demande::Rien;
        let cjk = crate::polices::cjk_disponible();
        let langue = self.langue_retenue(francais, cjk);
        let mut choisie = langue.clone();

        // Échap ferme, les flèches font défiler. Lus avant la fenêtre : une
        // touche consommée par un champ de saisie du classeur derrière ne
        // doit pas arriver ici, et il n'y en a pas tant que la fiche est
        // ouverte.
        ctx.input(|i| {
            if i.key_pressed(egui::Key::Escape) {
                demande = Demande::Fermer;
            } else if i.key_pressed(egui::Key::ArrowLeft) {
                demande = Demande::Precedente;
            } else if i.key_pressed(egui::Key::ArrowRight) {
                demande = Demande::Suivante;
            }
        });

        egui::Window::new("fiche_carte")
            .id(egui::Id::new("fiche_carte"))
            // Barre de titre d'egui retirée : elle n'apportait que le nom et
            // une croix de 14 points de côté, dont la zone cliquable ne fait
            // pas un pixel de plus — il fallait viser. La fenêtre étant
            // ancrée au centre, sa barre ne servait même pas à la déplacer.
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                if barre_de_titre(ui, fiche.nom_affiche(francais)) {
                    demande = Demande::Fermer;
                }
                ui.add_space(6.0);
                ui.horizontal_top(|ui| {
                    illustration(ui, cache, dossier_images, &fiche);
                    ui.add_space(16.0);
                    choisie = colonne(ui, &fiche, &langue, francais, cjk).langue;
                });
                ui.add_space(10.0);
                ui.separator();
                ui.horizontal(|ui| {
                    if ui
                        .button("◀")
                        .on_hover_text("Carte précédente (←)")
                        .clicked()
                    {
                        demande = Demande::Precedente;
                    }
                    if ui.button("▶").on_hover_text("Carte suivante (→)").clicked() {
                        demande = Demande::Suivante;
                    }
                    ui.add_space(12.0);
                    ui.label(egui::RichText::new(fiche.possession()).weak());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.small(egui::RichText::new("Échap pour fermer").weak());
                    });
                });
            });

        if choisie != langue {
            self.langue = Some(choisie);
        }
        demande
    }
}

/// Côté du bouton de fermeture, en points.
///
/// Vingt-huit, et non les quatorze de la croix d'egui. La règle usuelle veut
/// qu'une cible se vise sans effort à partir de vingt-quatre points ; en
/// dessous, on ne clique plus un bouton, on le cherche.
pub const COTE_FERMETURE: f32 = 28.0;

/// Le nom de la carte, et un bouton de fermeture qu'on n'a pas à viser.
///
/// Rend `true` si l'utilisateur a demandé la fermeture.
fn barre_de_titre(ui: &mut egui::Ui, nom: &str) -> bool {
    let mut fermer = false;
    ui.horizontal(|ui| {
        ui.heading(nom);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let bouton = egui::Button::new(egui::RichText::new(CROIX).size(16.0));
            if ui
                .add_sized([COTE_FERMETURE, COTE_FERMETURE], bouton)
                .on_hover_text("Fermer (Échap)")
                .clicked()
            {
                fermer = true;
            }
        });
    });
    fermer
}

/// La croix de fermeture.
///
/// `✕` (U+2715) et `✗` ne sont dans **aucune** des polices d'egui — la leçon
/// avait déjà été apprise sur le bouton d'effacement des filtres. `✖`
/// (U+2716) est dans emoji-icon-font, donc dans la famille proportionnelle.
pub const CROIX: &str = "✖";

/// L'illustration, à une définition où le texte imprimé se lit.
fn illustration(ui: &mut egui::Ui, cache: &mut Cache, dossier: &std::path::Path, fiche: &Fiche) {
    let taille = egui::vec2(LARGEUR_IMAGE, HAUTEUR_IMAGE);
    // Toujours la variante « possédée » : la fiche montre la carte, elle ne
    // rejoue pas l'atténuation du classeur. On vient précisément d'ouvrir
    // pour la regarder.
    let texture = fiche
        .fichier_image
        .as_ref()
        .and_then(|f| cache.texture(ui.ctx(), dossier, Cle::nouvelle(f, true, false)));
    let (rect, _) = ui.allocate_exact_size(taille, egui::Sense::hover());
    match texture {
        Some(t) => {
            egui::Image::from_texture(&t)
                .corner_radius(6.0)
                .paint_at(ui, rect);
        }
        None => {
            ui.painter()
                .rect_filled(rect, 6.0, ui.visuals().extreme_bg_color);
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                &fiche.set_code,
                egui::FontId::proportional(14.0),
                ui.visuals().weak_text_color(),
            );
        }
    }
}

/// Code, rareté, set, puis les statistiques.
fn entete(ui: &mut egui::Ui, fiche: &Fiche) {
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new(&fiche.set_code).monospace().strong());
        ui.label(egui::RichText::new("·").weak());
        ui.label(&fiche.rarete);
        if fiche.extended_art {
            ui.label(egui::RichText::new("OVERFRAME").small().strong());
        }
    });
    if !fiche.set_name.trim().is_empty() {
        ui.label(egui::RichText::new(&fiche.set_name).weak().small());
    }
    let stats = fiche.statistiques();
    if !stats.is_empty() {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(stats).monospace().small());
    }
}

/// Ce que dessine la colonne de droite. Les rectangles ne servent qu'aux
/// tests, qui mesurent la vraie colonne.
struct Colonne {
    /// La langue retenue après un éventuel clic sur un onglet.
    langue: String,
    /// La colonne entière.
    #[cfg_attr(not(test), allow(dead_code))]
    rect: egui::Rect,
    /// L'en-tête, les langues et le texte.
    #[cfg_attr(not(test), allow(dead_code))]
    texte: egui::Rect,
    /// Le bloc Cardmarket, s'il y en a un.
    #[cfg_attr(not(test), allow(dead_code))]
    cardmarket: Option<egui::Rect>,
}

/// La colonne de droite, **à la hauteur de l'illustration** : Cardmarket en
/// occupe le bas, l'en-tête, les langues et le texte le haut — le texte
/// défile dans ce qui reste.
///
/// # Pourquoi de bas en haut
///
/// Sous un texte court restait une grande place vide ; sous un texte long,
/// un bloc Cardmarket placé à la suite serait sorti de la fenêtre. Posé
/// d'abord, contre le bas d'une colonne de hauteur fixe, il est toujours au
/// même endroit, et le texte prend exactement la hauteur qu'il laisse. La
/// colonne se remplit donc de bas en haut ; le haut, lui, se lit de haut en
/// bas, comme avant.
fn colonne(ui: &mut egui::Ui, fiche: &Fiche, langue: &str, francais: bool, cjk: bool) -> Colonne {
    let taille = egui::vec2(LARGEUR_TEXTE, HAUTEUR_IMAGE);
    let mut choisie = langue.to_owned();
    let mut bloc = None;
    let mut texte = egui::Rect::NOTHING;
    let reponse =
        ui.allocate_ui_with_layout(taille, egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.set_min_size(taille);
            if let Some(offre) = &fiche.cardmarket {
                bloc = Some(
                    ui.scope(|ui| cardmarket(ui, offre, &fiche.set_code, francais))
                        .response
                        .rect,
                );
                ui.add_space(10.0);
            }
            texte = ui
                .with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    entete(ui, fiche);
                    ui.add_space(8.0);
                    choisie = onglets_de_langue(ui, fiche, langue, francais, cjk);
                    ui.add_space(6.0);
                    corps(ui, fiche, &choisie);
                })
                .response
                .rect;
        });
    Colonne {
        langue: choisie,
        rect: reponse.response.rect,
        texte,
        cardmarket: bloc,
    }
}

/// Cardmarket, en bas de la colonne : un bouton, les prix, leur date.
///
/// Le bouton ouvre la page de **la** version quand on sait laquelle c'est ;
/// sinon toutes les versions de la carte dans son set — la recherche de
/// Cardmarket, filtrée sur l'extension —, avec la version à ouvrir quand on
/// la connaît : la page de résultats trie par popularité, pas par V.n. Quand
/// la carte elle-même n'a pas été retrouvée (set non rapproché, nom
/// différent), la recherche de son numéro (`CORI-JP001`). Le lien s'ouvre
/// dans le navigateur ; l'application n'envoie aucune requête à
/// `www.cardmarket.com`.
///
/// Appelée dans une mise en page de bas en haut (cf. [`colonne`]) : les
/// lignes sont posées dans l'ordre inverse de la lecture.
fn cardmarket(ui: &mut egui::Ui, offre: &Offre, set_code: &str, francais: bool) {
    // La date et la portée des prix, quand il y en a un à dater.
    if offre.prix != PrixOffre::Aucun {
        ui.label(egui::RichText::new(note_des_prix(offre)).small().weak());
        ui.add_space(2.0);
    }
    ui.label(ligne_de_prix(offre));
    if let Some(texte) = indication(offre) {
        ui.add_space(2.0);
        ui.label(egui::RichText::new(texte).small());
    }
    ui.add_space(4.0);
    let url = offre.url(ygo_core::cardmarket::langue_du_site(francais));
    if ui
        .button(texte_du_bouton(offre, set_code))
        .on_hover_text(aide_du_lien(offre))
        .clicked()
    {
        ui.ctx().open_url(egui::OpenUrl::new_tab(url));
    }
    ui.add_space(4.0);
    ui.label(egui::RichText::new("Cardmarket").strong().small());
    ui.separator();
}

/// Le texte du bouton : la version quand on la tient, sinon toutes les
/// versions de la carte dans son set, sinon son numéro.
///
/// ```
/// use ygo_app::cardmarket::{Cible, Offre, PrixOffre, Raison};
/// use ygo_ui::fiche::texte_du_bouton;
/// let mut o = Offre { cible: Cible::Produit(1), version: Some("V.3 · Ultimate Rare".into()),
///     prix: PrixOffre::Aucun, raison: Raison::Ordonnee, extension: None,
///     prix_du: String::new() };
/// assert_eq!(texte_du_bouton(&o, "RA02-EN001"), "Voir sur Cardmarket · V.3 Ultimate Rare");
/// o.version = None;
/// assert_eq!(texte_du_bouton(&o, "RA02-EN001"), "Voir sur Cardmarket");
/// o.cible = Cible::Recherche { nom: "Rescue Cat".into(), id_extension: Some(5604) };
/// assert_eq!(texte_du_bouton(&o, "RA02-EN001"), "Voir toutes les versions · RA02");
/// o.cible = Cible::Recherche { nom: "Rescue Cat".into(), id_extension: None };
/// assert_eq!(texte_du_bouton(&o, "RA02-EN001"), "Chercher sur Cardmarket");
/// o.cible = Cible::Numero("CORI-JP001".into());
/// assert_eq!(texte_du_bouton(&o, "CORI-JP001"), "Chercher CORI-JP001 sur Cardmarket");
/// ```
#[must_use]
pub fn texte_du_bouton(offre: &Offre, set_code: &str) -> String {
    match &offre.cible {
        Cible::Produit(_) => match &offre.version {
            Some(version) => format!("Voir sur Cardmarket · {}", version.replacen(" · ", " ", 1)),
            None => "Voir sur Cardmarket".to_owned(),
        },
        Cible::Recherche {
            id_extension: Some(_),
            ..
        } => {
            let set = set_code.split('-').next().unwrap_or("").trim();
            if set.is_empty() {
                "Voir toutes les versions".to_owned()
            } else {
                format!("Voir toutes les versions · {set}")
            }
        }
        Cible::Recherche {
            id_extension: None, ..
        } => "Chercher sur Cardmarket".to_owned(),
        Cible::Numero(numero) => format!("Chercher {numero} sur Cardmarket"),
    }
}

/// Sous le bouton, quand il mène à toutes les versions : laquelle ouvrir si
/// on le sait, sinon pourquoi on ne le sait pas. Rien quand le bouton mène
/// à la bonne page.
///
/// ```
/// use ygo_app::cardmarket::{Cible, Offre, PrixOffre, Raison};
/// use ygo_ui::fiche::indication;
/// let mut o = Offre {
///     cible: Cible::Recherche { nom: "El Shaddoll Winda".into(), id_extension: Some(5989) },
///     version: Some("V.6 · Quarter Century Secret Rare (illustration 2)".into()),
///     prix: PrixOffre::Aucun, raison: Raison::IllustrationsSaisiesAPart, extension: None,
///     prix_du: String::new() };
/// assert_eq!(
///     indication(&o).unwrap(),
///     "Version à ouvrir : V.6 · Quarter Century Secret Rare (illustration 2)"
/// );
/// o.version = None;
/// o.raison = Raison::CarteAbsente;
/// assert_eq!(indication(&o).unwrap(), "Carte introuvable dans ce set chez Cardmarket");
/// o.cible = Cible::Numero("CORI-JP001".into());
/// o.raison = Raison::ExtensionInconnue;
/// assert_eq!(
///     indication(&o).unwrap(),
///     "Set non rapproché d'une extension Cardmarket : recherche par numéro"
/// );
/// o.cible = Cible::Produit(1);
/// assert_eq!(indication(&o), None);
/// ```
#[must_use]
pub fn indication(offre: &Offre) -> Option<String> {
    if offre.est_directe() {
        return None;
    }
    Some(match (&offre.version, &offre.cible) {
        (Some(version), _) => format!("Version à ouvrir : {version}"),
        (None, Cible::Numero(_)) => format!(
            "{} : recherche par numéro",
            majuscule(&offre.raison.explication())
        ),
        (None, _) => majuscule(&offre.raison.explication()),
    })
}

/// Les prix : tendance, plus bas, moyenne sur trente jours — ou la fourchette
/// des versions de la carte quand on ne sait pas laquelle est la bonne.
///
/// ```
/// use ygo_app::cardmarket::{Cible, Offre, Prix, PrixOffre, Raison};
/// use ygo_ui::fiche::ligne_de_prix;
/// let mut o = Offre { cible: Cible::Produit(1), version: None,
///     prix: PrixOffre::Exact(Prix { tendance: Some(33), bas: Some(7), moyenne30: Some(41) }),
///     raison: Raison::VersionUnique, extension: None, prix_du: String::new() };
/// assert_eq!(ligne_de_prix(&o), "Tendance 0,33 € · Plus bas 0,07 € · Moyenne 30 j 0,41 €");
/// o.prix = PrixOffre::Exact(Prix { tendance: None, bas: Some(7), moyenne30: None });
/// assert_eq!(ligne_de_prix(&o), "Plus bas 0,07 €");
/// o.prix = PrixOffre::Fourchette { min: 2, max: 399 };
/// assert_eq!(ligne_de_prix(&o), "De 0,02 € à 3,99 € selon la version");
/// o.prix = PrixOffre::Aucun;
/// assert_eq!(ligne_de_prix(&o), "Pas de prix publié");
/// o.raison = Raison::ExtensionInconnue;
/// assert_eq!(ligne_de_prix(&o), "Prix à lire sur Cardmarket");
/// ```
#[must_use]
pub fn ligne_de_prix(offre: &Offre) -> String {
    const AUCUN: &str = "Pas de prix publié";
    match offre.prix {
        PrixOffre::Exact(p) => {
            let parties: Vec<String> = [
                ("Tendance", p.tendance),
                ("Plus bas", p.bas),
                ("Moyenne 30 j", p.moyenne30),
            ]
            .into_iter()
            .filter_map(|(nom, centimes)| centimes.map(|c| format!("{nom} {}", euros(c.into()))))
            .collect();
            if parties.is_empty() {
                AUCUN.to_owned()
            } else {
                parties.join(" · ")
            }
        }
        PrixOffre::Fourchette { min, max } if min == max => {
            format!("{} selon la version", euros(min.into()))
        }
        PrixOffre::Fourchette { min, max } => {
            format!(
                "De {} à {} selon la version",
                euros(min.into()),
                euros(max.into())
            )
        }
        // La carte n'a pas été retrouvée : ses prix existent, on ne sait pas
        // lesquels sont les siens.
        PrixOffre::Aucun if offre.raison.carte_non_retrouvee() => {
            "Prix à lire sur Cardmarket".to_owned()
        }
        PrixOffre::Aucun => AUCUN.to_owned(),
    }
}

/// La date des prix, et ce qu'ils couvrent : sans cette mention, on lirait
/// le prix d'une carte française en parfait état.
///
/// ```
/// use ygo_app::cardmarket::{Cible, Offre, PrixOffre, Raison};
/// use ygo_ui::fiche::note_des_prix;
/// let mut o = Offre { cible: Cible::Produit(1), version: None, prix: PrixOffre::Aucun,
///     raison: Raison::VersionUnique, extension: None,
///     prix_du: "2026-10-22T02:44:58+0200".into() };
/// assert_eq!(
///     note_des_prix(&o),
///     "Prix du 22/10/2026 · toutes langues, éditions et états confondus"
/// );
/// o.prix_du = String::new();
/// assert_eq!(note_des_prix(&o), "Prix toutes langues, éditions et états confondus");
/// ```
#[must_use]
pub fn note_des_prix(offre: &Offre) -> String {
    const PORTEE: &str = "toutes langues, éditions et états confondus";
    match date_courte(&offre.prix_du) {
        Some(date) => format!("Prix du {date} · {PORTEE}"),
        None => format!("Prix {PORTEE}"),
    }
}

/// La première lettre en majuscule.
fn majuscule(texte: &str) -> String {
    let mut lettres = texte.chars();
    match lettres.next() {
        Some(premiere) => premiere.to_uppercase().chain(lettres).collect(),
        None => String::new(),
    }
}

/// L'info-bulle du lien : pourquoi ce lien, et de quand date le prix.
///
/// ```
/// use ygo_app::cardmarket::{Cible, Offre, PrixOffre, Raison};
/// let o = Offre { cible: Cible::Produit(1), version: None, prix: PrixOffre::Aucun,
///     raison: Raison::VersionUnique, extension: Some("Rarity Collection 5".into()),
///     prix_du: "2026-10-20T02:44:58+0200".into() };
/// let aide = ygo_ui::fiche::aide_du_lien(&o);
/// assert!(aide.contains("Rarity Collection 5"));
/// assert!(aide.contains("20/10/2026"));
/// ```
#[must_use]
pub fn aide_du_lien(offre: &ygo_app::cardmarket::Offre) -> String {
    let mut lignes = Vec::new();
    if let Some(ext) = &offre.extension {
        lignes.push(format!("Extension Cardmarket : {ext}"));
    }
    lignes.push(offre.raison.explication());
    if let Some(date) = date_courte(&offre.prix_du) {
        lignes.push(format!(
            "Prix du {date}, toutes langues, éditions et états confondus."
        ));
    }
    lignes.join("\n")
}

/// `2026-10-20T02:44:58+0200` → `20/10/2026`.
pub(crate) fn date_courte(horodatage: &str) -> Option<String> {
    let jour = horodatage.get(..10)?;
    let mut parties = jour.split('-');
    let (a, m, j) = (parties.next()?, parties.next()?, parties.next()?);
    Some(format!("{j}/{m}/{a}"))
}

/// Les langues que la fiche peut réellement **dessiner**.
///
/// `Fiche::langues` dit ce que la base contient ; ici on retire ce que les
/// polices chargées ne savent pas tracer. Sans ce filtre, un onglet
/// « 日本語 » s'affiche en carrés vides et, une fois choisi, montre un texte
/// en carrés vides : l'utilisateur croit à une carte abîmée alors que c'est
/// la police qui manque.
///
/// Le filtre ne s'applique **pas** si plus rien ne resterait : une carte qui
/// n'a que du japonais vaut mieux montrée mal que pas montrée du tout.
///
/// `cjk` est passé plutôt que lu : la disponibilité d'une police est un fait
/// de la machine, et une règle qui l'interroge elle-même ne se teste que sur
/// la machine qui exécute le test.
#[must_use]
pub fn langues_affichables(fiche: &Fiche, preferee: &str, cjk: bool) -> Vec<String> {
    let toutes = fiche.langues(preferee);
    if cjk {
        return toutes;
    }
    let lisibles: Vec<String> = toutes
        .iter()
        .filter(|l| !crate::polices::exige_des_ideogrammes(l))
        .cloned()
        .collect();
    if lisibles.is_empty() {
        toutes
    } else {
        lisibles
    }
}

/// Un bouton par langue offerte ; rend celle qui est retenue après le clic.
///
/// L'ordre suit la **préférence de l'écran**, pas la langue sélectionnée.
/// La première version rangeait la sélection en tête : cliquer sur « 日本語 »
/// le faisait sauter à gauche et poussait les autres d'un cran, si bien que
/// le deuxième clic tombait sur un onglet qu'on n'avait pas visé.
fn onglets_de_langue(
    ui: &mut egui::Ui,
    fiche: &Fiche,
    actuelle: &str,
    francais: bool,
    cjk: bool,
) -> String {
    let ancre = if francais { "fr" } else { "en" };
    let offertes = langues_affichables(fiche, ancre, cjk);
    if offertes.len() < 2 {
        return actuelle.to_owned();
    }
    let mut choisie = actuelle.to_owned();
    ui.horizontal_wrapped(|ui| {
        for langue in &offertes {
            let actif = langue == &choisie;
            if ui
                .selectable_label(actif, egui::RichText::new(nom_de_langue(langue)).small())
                .clicked()
            {
                choisie = langue.clone();
            }
        }
    });
    choisie
}

/// Le nom du texte et son effet, dans la langue retenue.
///
/// L'effet défile dans la hauteur qui reste au-dessus de Cardmarket (cf.
/// [`colonne`]).
fn corps(ui: &mut egui::Ui, fiche: &Fiche, langue: &str) {
    let Some(texte) = fiche.texte_en(langue) else {
        ui.label(egui::RichText::new(message_sans_texte(fiche.resolution)).weak());
        return;
    };
    if let Some(nom) = &texte.nom {
        if !nom.trim().is_empty() {
            ui.label(egui::RichText::new(nom).strong());
            ui.add_space(4.0);
        }
    }
    if let Some(effet) = &texte.effet {
        egui::ScrollArea::vertical()
            .max_height(ui.available_height())
            .show(ui, |ui| {
                ui.label(effet);
            });
    }
}

/// Ce qu'on dit quand il n'y a pas de texte, selon la raison.
///
/// « Aucun texte » tout court laisse croire à une panne. La raison change ce
/// qu'il y a à faire : une base absente se répare en initialisant, une carte
/// que la base ne connaît pas ne se répare pas du tout.
///
/// ```
/// use ygo_ui::fiche::message_sans_texte;
/// use ygo_app::fiche::Resolution;
/// assert!(message_sans_texte(Resolution::Aucune).contains("pas cette carte"));
/// assert!(message_sans_texte(Resolution::Uuid).contains("cette langue"));
/// ```
#[must_use]
pub fn message_sans_texte(resolution: Resolution) -> &'static str {
    match resolution {
        Resolution::Aucune => "La base des cartes ne connaît pas cette carte.",
        _ => "Pas de texte dans cette langue.",
    }
}

/// Le libellé d'une langue, pour l'onglet.
///
/// ```
/// use ygo_ui::fiche::nom_de_langue;
/// assert_eq!(nom_de_langue("fr"), "Français");
/// assert_eq!(nom_de_langue("zh-CN"), "中文");
/// // Une langue que la base gagnerait sans qu'on le sache reste lisible.
/// assert_eq!(nom_de_langue("xx"), "xx");
/// ```
#[must_use]
pub fn nom_de_langue(code: &str) -> &str {
    match code {
        "fr" => "Français",
        "en" => "English",
        "ja" => "日本語",
        "de" => "Deutsch",
        "es" => "Español",
        "it" => "Italiano",
        "pt" => "Português",
        "ko" => "한국어",
        "zh-CN" => "中文",
        "zh-TW" => "繁體",
        autre => autre,
    }
}

/// Le rang de la carte à montrer après un déplacement, borné à la page.
///
/// Aux extrémités on ne boucle pas : passer de la dernière carte à la
/// première donne l'impression d'avoir changé de page alors qu'on n'a pas
/// bougé. On s'arrête, et la fiche reste sur place.
///
/// ```
/// use ygo_ui::fiche::voisine;
/// assert_eq!(voisine(0, 9, 1), 1);
/// assert_eq!(voisine(8, 9, 1), 8, "pas de bouclage à la fin");
/// assert_eq!(voisine(0, 9, -1), 0, "ni au début");
/// assert_eq!(voisine(4, 0, -1), 4, "une page vide ne déplace rien");
/// ```
#[must_use]
pub fn voisine(actuel: usize, total: usize, pas: isize) -> usize {
    if total == 0 {
        return actuel;
    }
    let vise = actuel as isize + pas;
    if vise < 0 {
        return actuel;
    }
    let vise = vise as usize;
    if vise >= total {
        return actuel;
    }
    vise
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;
    use ygo_app::fiche::Texte;

    fn texte(langue: &str, effet: Option<&str>) -> Texte {
        Texte {
            langue: langue.to_owned(),
            nom: Some(format!("nom {langue}")),
            effet: effet.map(str::to_owned),
        }
    }

    fn fiche(textes: Vec<Texte>) -> Fiche {
        Fiche {
            rowid: 1,
            nom: "Blue-Eyes".into(),
            nom_fr: String::new(),
            set_code: "LOCR-JP001".into(),
            rarete: "Ultra Rare".into(),
            set_name: "Legacy of Creation".into(),
            card_type: "Effect Monster".into(),
            atk: Some(3000),
            def: Some(2500),
            level: Some(8),
            attribute: "LIGHT".into(),
            race: "Dragon".into(),
            quantite: 1,
            qualite: None,
            edition: None,
            extended_art: false,
            card_image_id: 42,
            fichier_image: None,
            resolution: Resolution::Uuid,
            textes,
            cardmarket: None,
        }
    }

    /// La langue choisie suit d'une carte à l'autre, tant qu'elle est offerte.
    ///
    /// C'est ce qui rend le feuilletage d'un classeur japonais supportable :
    /// on bascule une fois, pas à chaque carte.
    #[test]
    fn la_langue_choisie_suit_d_une_carte_a_l_autre() {
        let mut etat = EtatFiche::default();
        etat.ouvrir(fiche(vec![
            texte("de", Some("Wirkung")),
            texte("en", Some("Effect")),
        ]));
        etat.langue = Some("de".into());
        assert_eq!(etat.langue_retenue(true, true), "de");

        // Carte suivante, qui a aussi de l'allemand : on y reste.
        etat.ouvrir(fiche(vec![
            texte("de", Some("Wirkung 2")),
            texte("en", Some("Effect 2")),
        ]));
        assert_eq!(etat.langue_retenue(true, true), "de", "le choix survit");

        // Mais une carte sans allemand ne laisse pas la fiche vide.
        etat.ouvrir(fiche(vec![
            texte("en", Some("Effect")),
            texte("ja", Some("効果")),
        ]));
        assert_eq!(
            etat.langue_retenue(true, true),
            "en",
            "on repart de la préférence quand le choix n'est plus offert"
        );
    }

    /// Et même si la langue survivait, une langue non offerte est écartée.
    ///
    /// Ceinture et bretelles : `ouvrir` oublie déjà, mais `langue_retenue`
    /// est la fonction qui décide, et c'est elle qui doit tenir la garantie.
    #[test]
    fn une_langue_non_offerte_ne_vide_pas_la_fiche() {
        let mut etat = EtatFiche::default();
        etat.ouvrir(fiche(vec![texte("en", Some("Effect"))]));
        etat.langue = Some("de".into());
        assert_eq!(etat.langue_retenue(true, true), "en");
    }

    /// Fiche fermée : la langue retenue reste la préférence, sans paniquer.
    #[test]
    fn une_fiche_fermee_rend_la_preference() {
        let etat = EtatFiche::default();
        assert_eq!(etat.langue_retenue(true, true), "fr");
        assert_eq!(etat.langue_retenue(false, true), "en");
        assert!(!etat.est_ouverte());
        assert!(etat.carte().is_none());
    }

    /// Une carte sans aucun texte ne fait pas planter le choix de langue.
    #[test]
    fn une_carte_sans_texte_retombe_sur_la_preference() {
        let mut etat = EtatFiche::default();
        etat.ouvrir(fiche(Vec::new()));
        assert_eq!(etat.langue_retenue(true, true), "fr");
        assert_eq!(etat.langue_retenue(false, true), "en");
    }

    /// Le message d'absence dit ce qui se passe, pas seulement qu'il ne se
    /// passe rien.
    #[test]
    fn le_message_d_absence_distingue_les_deux_causes() {
        assert_ne!(
            message_sans_texte(Resolution::Aucune),
            message_sans_texte(Resolution::Uuid),
            "carte inconnue et langue manquante ne se réparent pas pareil"
        );
        for voie in [Resolution::Uuid, Resolution::ParImage, Resolution::ParCode] {
            assert_eq!(
                message_sans_texte(voie),
                message_sans_texte(Resolution::Uuid),
                "la carte est trouvée : seule la langue manque"
            );
        }
    }

    /// Le déplacement s'arrête aux bords au lieu de boucler.
    #[test]
    fn le_deplacement_s_arrete_aux_bords() {
        assert_eq!(voisine(0, 9, 1), 1);
        assert_eq!(voisine(7, 9, 1), 8);
        assert_eq!(voisine(8, 9, 1), 8, "fin de page");
        assert_eq!(voisine(0, 9, -1), 0, "début de page");
        assert_eq!(voisine(3, 9, -1), 2);
        assert_eq!(voisine(0, 0, 1), 0, "page vide");
        // Un rang hors page ne déplace pas plus loin.
        assert_eq!(voisine(20, 9, 1), 20);
    }

    /// L'ordre des onglets ne dépend pas de celui qui est sélectionné.
    ///
    /// Vu à l'écran avant d'être écrit ici : cliquer sur « 日本語 » le
    /// ramenait en première position et décalait tous les autres. Le
    /// deuxième clic tombait alors sur un onglet qu'on n'avait pas visé.
    #[test]
    fn choisir_une_langue_ne_reordonne_pas_les_onglets() {
        let f = fiche(vec![
            texte("fr", Some("Effet")),
            texte("en", Some("Effect")),
            texte("ja", Some("効果")),
        ]);
        let ordre = langues_affichables(&f, "fr", true);
        assert_eq!(ordre, vec!["fr", "en", "ja"]);
        // L'ancre reste la préférence de l'écran quelle que soit la
        // sélection : c'est elle qui range les onglets.
        assert_eq!(langues_affichables(&f, "fr", true), ordre);
        // Et ranger par la sélection donnerait, lui, un autre ordre — c'est
        // bien ce qu'on refuse.
        assert_ne!(langues_affichables(&f, "ja", true), ordre);
    }

    /// Sans police à idéogrammes, ces langues ne sont pas offertes…
    ///
    /// …sauf s'il ne resterait rien : une carte qui n'a que du japonais vaut
    /// mieux montrée mal que pas montrée du tout.
    #[test]
    fn sans_police_a_ideogrammes_ces_langues_sont_ecartees() {
        let melange = fiche(vec![
            texte("fr", Some("Effet")),
            texte("ja", Some("効果")),
            texte("ko", Some("효과")),
            texte("zh-CN", Some("效果")),
        ]);
        assert_eq!(langues_affichables(&melange, "fr", false), vec!["fr"]);
        assert_eq!(
            langues_affichables(&melange, "fr", true),
            vec!["fr", "ja", "ko", "zh-CN"],
            "avec la police, tout est offert"
        );

        // Rien d'autre que du japonais : on le montre quand même.
        let seulement_ja = fiche(vec![texte("ja", Some("効果"))]);
        assert_eq!(
            langues_affichables(&seulement_ja, "fr", false),
            vec!["ja"],
            "mieux vaut des carrés que le vide"
        );
    }

    /// Le bouton de fermeture se vise sans effort.
    ///
    /// # Le défaut que ce test attrape
    ///
    /// La croix de la barre de titre d'egui est allouée à
    /// `spacing().icon_width`, soit **14 points de côté**, et sa zone
    /// cliquable ne fait pas un pixel de plus (`close_button` interagit
    /// exactement sur ce rectangle). L'utilisateur l'a dit ainsi : « il faut
    /// cliquer au pixel près ».
    ///
    /// On mesure donc le rectangle **réellement alloué**, pas la constante :
    /// `add_sized` peut être contrarié par la mise en page, et c'est ce que
    /// la souris rencontre qui compte.
    #[test]
    fn le_bouton_de_fermeture_se_vise_sans_effort() {
        // On appelle la VRAIE barre de titre. Une première version
        // reconstruisait le bouton dans le test : la mutation qui remplaçait
        // `add_sized` par `add` y survivait tranquillement, puisque le test
        // gardait son propre `add_sized`. C'est la troisième fois que ce
        // piège se referme dans ce projet.
        let mut hauteur = 0.0_f32;
        avec_polices(|ui| {
            let _ = barre_de_titre(ui, "Obelisk the Tormentor");
            hauteur = ui.min_rect().height();
        });
        assert!(
            hauteur >= COTE_FERMETURE,
            "la barre fait {hauteur} points de haut : le bouton n'occupe pas \
             les {COTE_FERMETURE} qu'on lui a demandés"
        );
        // Et nettement plus que la croix qu'on remplace.
        assert!(
            hauteur >= 2.0 * 14.0,
            "la croix d'egui faisait 14 points de côté ; {hauteur} n'est pas un progrès"
        );
    }

    /// La croix employée existe dans une police que la fenêtre consulte.
    ///
    /// `✕` (U+2715) et `✗` (U+2717) ne sont dans aucune des quatre polices
    /// d'egui : les employer donnerait un carré vide, comme les chevrons
    /// `▸`/`▾` du tableau des statistiques. `✖` est U+2716, dans
    /// emoji-icon-font.
    #[test]
    fn la_croix_est_dans_une_police_disponible() {
        assert_eq!(CROIX.chars().count(), 1);
        assert_eq!(
            CROIX.chars().next().map(u32::from),
            Some(0x2716),
            "U+2715 et U+2717 ne sont dans aucune police d'egui"
        );
    }

    /// Une offre Cardmarket complète, pour les tests de la colonne.
    fn offre() -> Offre {
        Offre {
            cible: Cible::Produit(875_196),
            version: Some("V.4 · Secret Parallel Rare".into()),
            prix: PrixOffre::Exact(ygo_app::cardmarket::Prix {
                tendance: Some(15_113),
                bas: Some(11_999),
                moyenne30: Some(16_000),
            }),
            raison: ygo_app::cardmarket::Raison::Ordonnee,
            extension: Some("Limit Over Collection: The Heroes".into()),
            prix_du: "2026-10-22T02:44:58+0200".into(),
        }
    }

    /// La vraie colonne, mesurée : `(colonne, texte, bloc Cardmarket)`.
    fn mesurer(f: &Fiche) -> (egui::Rect, egui::Rect, Option<egui::Rect>) {
        let mut mesure = None;
        avec_polices(|ui| {
            let c = colonne(ui, f, "fr", true, true);
            mesure = Some((c.rect, c.texte, c.cardmarket));
        });
        mesure.unwrap()
    }

    /// Cardmarket tient le bas de la colonne, la colonne garde la hauteur de
    /// l'illustration, et le texte ne passe jamais dessous — qu'il soit long
    /// ou court.
    ///
    /// # Les défauts que ce test attrape
    ///
    /// Un bloc posé à la suite du texte, comme la première idée le voulait,
    /// montait et descendait avec lui. Et un effet long qui ne défilerait
    /// plus passerait sous le bloc sans que la colonne change de taille :
    /// seule la mesure du texte lui-même le voit.
    #[test]
    fn cardmarket_tient_le_bas_de_la_colonne() {
        let long = "Une ligne d'effet qui n'en finit pas. ".repeat(120);
        for effet in [long.as_str(), "Court."] {
            let mut f = fiche(vec![texte("fr", Some(effet))]);
            f.cardmarket = Some(offre());
            let (rect, texte, bloc) = mesurer(&f);
            assert!(
                (rect.height() - HAUTEUR_IMAGE).abs() < 1.0,
                "la colonne fait {} points de haut au lieu de {HAUTEUR_IMAGE}",
                rect.height()
            );
            let bloc = bloc.unwrap();
            assert!(
                (bloc.bottom() - rect.bottom()).abs() < 1.0,
                "Cardmarket n'est pas en bas : {bloc:?} dans {rect:?}"
            );
            assert!(
                texte.bottom() <= bloc.top() + 0.5,
                "le texte passe sous Cardmarket : {texte:?} / {bloc:?}"
            );
        }
    }

    /// Sans fichiers Cardmarket, rien n'est réservé : la colonne garde sa
    /// hauteur, et le texte en dispose.
    #[test]
    fn sans_cardmarket_rien_n_est_reserve() {
        let f = fiche(vec![texte("fr", Some("Effet"))]);
        let (rect, _, bloc) = mesurer(&f);
        assert!(bloc.is_none());
        assert!((rect.height() - HAUTEUR_IMAGE).abs() < 1.0);
    }

    /// Un `Ui` de test **avec ses polices** — voir la note de
    /// `statistiques.rs`, où le même piège avait rendu deux tests muets.
    fn avec_polices(mut contenu: impl FnMut(&mut egui::Ui)) {
        let ctx = egui::Context::default();
        let entree = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1400.0, 900.0),
            )),
            ..Default::default()
        };
        for _ in 0..2 {
            ctx.run_ui(entree.clone(), |ui| contenu(ui))
                .drop_without_applying_deltas();
        }
    }

    /// Les libellés de langue existent pour toutes celles du référentiel.
    ///
    /// Une langue rendue par son code brut au milieu de noms écrits en
    /// toutes lettres se remarque ; le test le remarque avant l'utilisateur.
    #[test]
    fn chaque_langue_du_referentiel_a_son_libelle() {
        for code in ygo_app::fiche::ORDRE_LANGUES {
            assert_ne!(
                nom_de_langue(code),
                code,
                "{code} n'a pas de libellé lisible"
            );
        }
    }
}
