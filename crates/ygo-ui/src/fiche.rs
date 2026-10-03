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
                    ui.vertical(|ui| {
                        ui.set_width(LARGEUR_TEXTE);
                        entete(ui, &fiche);
                        ui.add_space(8.0);
                        choisie = onglets_de_langue(ui, &fiche, &langue, francais, cjk);
                        ui.add_space(6.0);
                        corps(ui, &fiche, &choisie);
                    });
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
    let taille = egui::vec2(LARGEUR_IMAGE, LARGEUR_IMAGE * 86.0 / 59.0);
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
            .max_height(LARGEUR_IMAGE * 86.0 / 59.0 - 120.0)
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
