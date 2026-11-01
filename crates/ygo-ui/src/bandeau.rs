// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Le bandeau d'une carte du classeur.
//!
//! Trois lignes au bas de la carte :
//!
//! ```text
//! ┌─────────────────────────────────┐
//! │ Chaos Sombre                    │
//! │ CORI-JP001 · Super Rare         │
//! │ PLAYSET      3,88 €   OVERFRAME │
//! └─────────────────────────────────┘
//! ```
//!
//! # D'où il vient
//!
//! Le bandeau n'apparaissait qu'au survol, et cachait alors le prix et les
//! badges du bas : le nom et le prix ne se voyaient jamais ensemble. Souhait
//! du 2026-11-01 : tout réunir dans un seul bandeau, affiché en permanence.
//! Les badges et le prix n'ont pas bougé — le fond s'est glissé derrière eux,
//! et le numéro partage désormais sa ligne avec la rareté, ce qui garde trois
//! lignes.
//!
//! La case « Bandeau toujours visible » du menu « ⋯ » rend l'usage d'avant :
//! les pastilles seules au repos, le bandeau au survol.
//!
//! # Trois formes
//!
//! Au repos, le bandeau ne doit couvrir que la boîte de texte de la carte —
//! illisible à la taille d'une vignette —, jamais l'illustration. Il prend la
//! plus complète des formes qui tient dans [`PART_MAX`] de la hauteur :
//!
//! | Forme | Lignes | Hauteur | Carte, à partir de |
//! |---|---|---|---|
//! | [`Forme::Complete`] | nom · numéro et rareté · badges et prix | 52 pt | 193 pt de haut |
//! | [`Forme::Compacte`] | rareté abrégée et nom · badges et prix | 38 pt | 141 pt de haut |
//! | [`Forme::Pastilles`] | badges et prix, sans fond | — | — |
//!
//! Au survol, il est toujours complet : c'est là qu'une petite carte montre ce
//! qu'elle tait au repos. Une forme a toujours la même hauteur, avec ou sans
//! badges ni prix : les bandeaux d'une rangée restent alignés.
//!
//! # Sur une carte étroite
//!
//! Le prix garde le centre ([`disposer`]). Quand le mot entier d'un badge
//! toucherait le prix ou l'autre badge, les deux passent à leur sigle (`PS`,
//! `OF`), puis à une simple pastille de leur couleur ; un badge s'efface s'il
//! n'y a même plus la place de la pastille. Le nom se coupe sur « … » ; la
//! rareté s'abrège avant de se couper. Tout ce qui a été raccourci se lit en
//! entier dans l'infobulle de la carte, dont [`dessiner`] rend le texte.

use std::sync::Arc;

use eframe::egui;

/// Hauteur des badges et du prix, sur la dernière ligne.
pub const HAUTEUR_BADGE: f32 = 15.0;
/// Écart entre la ligne des badges et les bords de la carte.
pub const RETRAIT_BADGE: f32 = 4.0;
/// Écart minimal entre deux éléments de la ligne des badges.
const ECART: f32 = 2.0;
/// Retrait du texte depuis les bords de la carte.
const MARGE_TEXTE: f32 = 6.0;
/// Espace au-dessus de la première ligne de texte.
const HAUT_TEXTE: f32 = 4.0;
/// Hauteur réservée à la ligne du nom, en 11 points.
const LIGNE_NOM: f32 = 14.0;
/// Hauteur réservée à une ligne de détail, en 10 points.
const LIGNE_DETAIL: f32 = 13.0;
/// Espace entre le texte et la ligne des badges.
const AVANT_BADGES: f32 = 2.0;

/// La part de la hauteur d'une carte que le bandeau peut couvrir au repos.
///
/// Sous l'illustration, il ne reste que la ligne du numéro imprimé et la
/// boîte de texte : sur les scans de l'installation, l'illustration s'arrête
/// vers 72 % de la hauteur de la carte.
pub const PART_MAX: f32 = 0.27;

/// Opacité du fond — celle du bandeau de survol d'origine.
const OPACITE_FOND: u8 = 205;
/// Arrondi des coins bas du fond : celui des images.
const ARRONDI: u8 = 4;

/// La forme du bandeau d'une carte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Forme {
    /// Le nom, le numéro et la rareté, puis les badges et le prix.
    Complete,
    /// La rareté abrégée et le nom sur une ligne, puis les badges et le prix.
    Compacte,
    /// Les badges et le prix seuls, sans fond — l'affichage d'avant le
    /// bandeau permanent.
    Pastilles,
}

impl Forme {
    /// Hauteur du fond, en points ; zéro sans fond.
    ///
    /// Elle ne dépend que de la forme, jamais du contenu : une carte sans
    /// badge ni prix garde un bandeau aligné sur ses voisines.
    #[must_use]
    pub fn hauteur(self) -> f32 {
        let badges = AVANT_BADGES + HAUTEUR_BADGE + RETRAIT_BADGE;
        match self {
            Self::Complete => HAUT_TEXTE + LIGNE_NOM + LIGNE_DETAIL + badges,
            Self::Compacte => HAUT_TEXTE + LIGNE_DETAIL + badges,
            Self::Pastilles => 0.0,
        }
    }

    /// La forme d'une carte au repos : la plus complète qui tient dans
    /// [`PART_MAX`] de sa hauteur.
    ///
    /// ```
    /// use ygo_ui::bandeau::Forme;
    /// // La carte habituelle : 260 points de large, 380 de haut.
    /// assert_eq!(Forme::au_repos(380.0), Forme::Complete);
    /// assert_eq!(Forme::au_repos(160.0), Forme::Compacte);
    /// // La plus petite carte permise : 90 points de large.
    /// assert_eq!(Forme::au_repos(131.0), Forme::Pastilles);
    /// ```
    #[must_use]
    pub fn au_repos(hauteur_carte: f32) -> Self {
        let place = hauteur_carte * PART_MAX;
        [Self::Complete, Self::Compacte]
            .into_iter()
            .find(|forme| forme.hauteur() <= place)
            .unwrap_or(Self::Pastilles)
    }
}

/// Les deux badges de la ligne du bas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sorte {
    /// Trois exemplaires ou plus — à gauche.
    Playset,
    /// Art étendu — à droite.
    Overframe,
}

impl Sorte {
    /// Le libellé entier.
    #[must_use]
    pub fn libelle(self) -> &'static str {
        match self {
            Self::Playset => "PLAYSET",
            Self::Overframe => "OVERFRAME",
        }
    }

    /// Le sigle, quand le mot entier ne tient pas : les initiales des deux
    /// mots — *play set*, *over frame*.
    #[must_use]
    pub fn sigle(self) -> &'static str {
        match self {
            Self::Playset => "PS",
            Self::Overframe => "OF",
        }
    }

    /// Ce qu'écrit un badge de cette taille — rien en pastille.
    #[must_use]
    pub fn texte(self, taille: Taille) -> &'static str {
        match taille {
            Taille::Entiere => self.libelle(),
            Taille::Sigle => self.sigle(),
            Taille::Pastille => "",
        }
    }

    /// Vert du badge Playset — `C["playset"]` ; violet du badge Overframe —
    /// `C["overframe"]`.
    fn couleur(self) -> egui::Color32 {
        match self {
            Self::Playset => egui::Color32::from_rgb(64, 150, 108),
            Self::Overframe => egui::Color32::from_rgb(138, 92, 200),
        }
    }

    /// La largeur du badge à cette taille.
    ///
    /// Une mesure fixe plutôt que celle de la police : la disposition se
    /// calcule ainsi sans contexte egui, et s'éprouve.
    #[must_use]
    pub fn largeur(self, taille: Taille) -> f32 {
        if taille == Taille::Pastille {
            return LARGEUR_PASTILLE;
        }
        #[allow(clippy::cast_precision_loss)]
        let lettres = self.texte(taille).chars().count() as f32;
        lettres * 5.6 + 10.0
    }
}

/// Largeur d'un badge réduit à sa couleur.
const LARGEUR_PASTILLE: f32 = 10.0;

/// La taille d'un badge, de la plus lisible à la plus étroite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Taille {
    /// Le mot entier : `PLAYSET`, `OVERFRAME`.
    Entiere,
    /// Le sigle : `PS`, `OF`.
    Sigle,
    /// La couleur seule.
    Pastille,
}

/// Un badge posé sur la ligne du bas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Badge {
    /// La zone qu'il occupe.
    pub zone: egui::Rect,
    /// Sa taille — moindre qu'[`Taille::Entiere`] faute de place.
    pub taille: Taille,
}

/// La ligne du bas d'une carte : où vont les badges et le prix.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Disposition {
    /// PLAYSET, à gauche.
    pub playset: Option<Badge>,
    /// Le prix, au centre.
    pub prix: Option<egui::Rect>,
    /// OVERFRAME, à droite.
    pub overframe: Option<Badge>,
}

/// Dispose la ligne du bas : le prix au centre, PLAYSET à gauche, OVERFRAME
/// à droite — les places d'origine des badges.
///
/// Le prix est prioritaire : il garde le centre tant qu'il tient dans la
/// carte. Les badges passent à leur sigle, puis à leur seule couleur, si le
/// mot entier touche le prix ou l'autre badge — les deux à la fois, car un
/// mot entier à gauche et un sigle à droite se liraient comme deux sortes de
/// badges. Un badge qui ne tient pas même en pastille n'est pas posé.
///
/// `largeur_prix` : celle de la pastille du prix, `None` sans prix.
#[must_use]
pub fn disposer(
    carte: egui::Rect,
    largeur_prix: Option<f32>,
    playset: bool,
    overframe: bool,
) -> Disposition {
    let bas = carte.bottom() - RETRAIT_BADGE;
    let gauche = carte.left() + RETRAIT_BADGE;
    let droite = carte.right() - RETRAIT_BADGE;
    let entre = |de: f32, a: f32| {
        egui::Rect::from_min_max(egui::pos2(de, bas - HAUTEUR_BADGE), egui::pos2(a, bas))
    };
    // Dans la carte, et à l'écart de chacune des autres zones de la ligne.
    let tient = |zone: egui::Rect, autres: &[Option<egui::Rect>]| {
        zone.left() >= gauche
            && zone.right() <= droite
            && autres.iter().flatten().all(|autre| {
                zone.right() + ECART <= autre.left() || autre.right() + ECART <= zone.left()
            })
    };
    let poser = |sorte: Sorte, present: bool, taille: Taille| {
        present.then(|| {
            let largeur = sorte.largeur(taille);
            let zone = match sorte {
                Sorte::Playset => entre(gauche, gauche + largeur),
                Sorte::Overframe => entre(droite - largeur, droite),
            };
            Badge { zone, taille }
        })
    };

    let centre = carte.center().x;
    let prix = largeur_prix
        .map(|largeur| entre(centre - largeur / 2.0, centre + largeur / 2.0))
        .filter(|zone| tient(*zone, &[]));

    for taille in [Taille::Entiere, Taille::Sigle, Taille::Pastille] {
        let pl = poser(Sorte::Playset, playset, taille);
        let of = poser(Sorte::Overframe, overframe, taille);
        let pl_tient = pl.is_none_or(|b| tient(b.zone, &[prix, of.map(|o| o.zone)]));
        let of_tient = of.is_none_or(|b| tient(b.zone, &[prix, pl.map(|p| p.zone)]));
        if pl_tient && of_tient {
            return Disposition {
                playset: pl,
                prix,
                overframe: of,
            };
        }
    }
    // Même en pastilles, ils ne tiennent pas ensemble : chacun reste s'il
    // laisse le prix libre — PLAYSET, à gauche, d'abord.
    let pl = poser(Sorte::Playset, playset, Taille::Pastille).filter(|b| tient(b.zone, &[prix]));
    let of = poser(Sorte::Overframe, overframe, Taille::Pastille)
        .filter(|b| tient(b.zone, &[prix, pl.map(|p| p.zone)]));
    Disposition {
        playset: pl,
        prix,
        overframe: of,
    }
}

/// Ce que le bandeau montre d'une carte.
#[derive(Debug, Clone, Copy)]
pub struct Contenu<'a> {
    /// Le nom, dans la langue du classeur.
    pub nom: &'a str,
    /// Le numéro.
    pub set_code: &'a str,
    /// Le libellé de rareté.
    pub rarete: &'a str,
    /// Trois exemplaires ou plus.
    pub playset: bool,
    /// Art étendu.
    pub overframe: bool,
    /// Le prix d'un exemplaire, déjà mis en forme ; `None` s'il ne s'affiche
    /// pas.
    pub prix: Option<&'a str>,
    /// La carte est-elle possédée ? Sinon le bandeau s'atténue avec elle.
    pub possedee: bool,
}

/// Dessine le bandeau d'une carte sous la forme demandée.
///
/// `vif` : la carte est survolée — le texte reprend son éclat même sur une
/// carte non possédée, comme l'image.
///
/// Rend le texte entier du bandeau ([`description`]) quand quelque chose a dû
/// être raccourci — nom coupé, rareté abrégée, badge réduit ou effacé, prix
/// sans place : c'est l'infobulle de la carte. `None` si tout se lit.
pub fn dessiner(
    painter: &egui::Painter,
    carte: egui::Rect,
    contenu: &Contenu<'_>,
    forme: Forme,
    vif: bool,
) -> Option<String> {
    let mut raccourci = false;

    // ── Le fond et le texte ─────────────────────────────────────────────
    if forme != Forme::Pastilles {
        let fond = egui::Rect::from_min_max(
            egui::pos2(carte.left(), carte.bottom() - forme.hauteur()),
            carte.right_bottom(),
        );
        painter.rect_filled(
            fond,
            egui::CornerRadius {
                nw: 0,
                ne: 0,
                sw: ARRONDI,
                se: ARRONDI,
            },
            egui::Color32::from_black_alpha(OPACITE_FOND),
        );
        // Une carte non possédée est délavée : son texte aussi, sauf au survol.
        let (clair, sourd) = if contenu.possedee || vif {
            (egui::Color32::from_gray(232), egui::Color32::from_gray(190))
        } else {
            (egui::Color32::from_gray(165), egui::Color32::from_gray(135))
        };
        let largeur = (carte.width() - 2.0 * MARGE_TEXTE).max(0.0);
        let coin = egui::pos2(carte.left() + MARGE_TEXTE, fond.top() + HAUT_TEXTE);
        match forme {
            Forme::Complete => {
                let nom = ligne(painter, contenu.nom, 11.0, clair, largeur);
                raccourci |= nom.elided;
                painter.galley(coin, nom, clair);

                let mut detail = ligne(
                    painter,
                    &numero_et_rarete(contenu.set_code, contenu.rarete),
                    10.0,
                    sourd,
                    largeur,
                );
                if detail.elided {
                    // La rareté s'abrège avant que la ligne ne se coupe.
                    let abregee = crate::artworks::abreviation(contenu.rarete);
                    detail = ligne(
                        painter,
                        &numero_et_rarete(contenu.set_code, &abregee),
                        10.0,
                        sourd,
                        largeur,
                    );
                    raccourci = true;
                }
                painter.galley(coin + egui::vec2(0.0, LIGNE_NOM), detail, sourd);
            }
            Forme::Compacte => {
                // Le numéro cède sa place : il est imprimé sur la carte.
                let abregee = crate::artworks::abreviation(contenu.rarete);
                let texte = joindre(&[&abregee, contenu.nom]);
                let galee = ligne(painter, &texte, 10.0, clair, largeur);
                painter.galley(coin, galee, clair);
                raccourci = true;
            }
            Forme::Pastilles => {}
        }
    }

    // ── La ligne des badges et du prix ──────────────────────────────────
    let prix = contenu.prix.map(|texte| {
        painter.layout_no_wrap(
            texte.to_owned(),
            egui::FontId::proportional(10.0),
            if contenu.possedee {
                egui::Color32::from_gray(240)
            } else {
                egui::Color32::from_gray(150)
            },
        )
    });
    let disposition = disposer(
        carte,
        prix.as_ref().map(|galee| galee.size().x + 12.0),
        contenu.playset,
        contenu.overframe,
    );
    for (sorte, present, pose) in [
        (Sorte::Playset, contenu.playset, disposition.playset),
        (Sorte::Overframe, contenu.overframe, disposition.overframe),
    ] {
        match pose {
            Some(badge) => {
                painter.rect_filled(badge.zone, 3.0, sorte.couleur());
                let texte = sorte.texte(badge.taille);
                if !texte.is_empty() {
                    painter.text(
                        badge.zone.center(),
                        egui::Align2::CENTER_CENTER,
                        texte,
                        egui::FontId::proportional(9.0),
                        egui::Color32::WHITE,
                    );
                }
                raccourci |= badge.taille != Taille::Entiere;
            }
            None => raccourci |= present,
        }
    }
    match (prix, disposition.prix) {
        (Some(galee), Some(zone)) => {
            // Sans bandeau, la pastille fait le fond qui manque.
            if forme == Forme::Pastilles {
                let fond =
                    egui::Color32::from_black_alpha(if contenu.possedee { 200 } else { 130 });
                painter.rect_filled(zone, HAUTEUR_BADGE / 2.0, fond);
            }
            painter.galley(
                zone.center() - galee.size() / 2.0,
                galee,
                egui::Color32::WHITE,
            );
        }
        (Some(_), None) => raccourci = true,
        (None, _) => {}
    }

    raccourci.then(|| description(contenu))
}

/// Une ligne de texte, coupée sur « … » au-delà de `largeur`.
fn ligne(
    painter: &egui::Painter,
    texte: &str,
    taille: f32,
    couleur: egui::Color32,
    largeur: f32,
) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(
        texte.to_owned(),
        egui::FontId::proportional(taille),
        couleur,
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(largeur);
    painter.layout_job(job)
}

/// Les parties non vides, séparées d'un point médian.
fn joindre(parties: &[&str]) -> String {
    parties
        .iter()
        .map(|partie| partie.trim())
        .filter(|partie| !partie.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

/// « numéro · rareté » — le numéro seul quand la rareté est vide.
///
/// ```
/// use ygo_ui::bandeau::numero_et_rarete;
/// assert_eq!(numero_et_rarete("CORI-JP001", "Super Rare"), "CORI-JP001 · Super Rare");
/// assert_eq!(numero_et_rarete("CORI-JP001", " "), "CORI-JP001");
/// ```
#[must_use]
pub fn numero_et_rarete(set_code: &str, rarete: &str) -> String {
    joindre(&[set_code, rarete])
}

/// Le texte entier du bandeau, pour l'infobulle d'une carte dont quelque
/// chose a été raccourci : une ligne par ligne du bandeau, sans les vides.
///
/// ```
/// use ygo_ui::bandeau::{description, Contenu};
/// let carte = Contenu {
///     nom: "Chaos Sombre",
///     set_code: "CORI-JP001",
///     rarete: "Super Rare",
///     playset: true,
///     overframe: false,
///     prix: Some("3,88 €"),
///     possedee: true,
/// };
/// assert_eq!(
///     description(&carte),
///     "Chaos Sombre\nCORI-JP001 · Super Rare\nPLAYSET · 3,88 €"
/// );
/// let nue = Contenu { playset: false, prix: None, ..carte };
/// assert_eq!(description(&nue), "Chaos Sombre\nCORI-JP001 · Super Rare");
/// ```
#[must_use]
pub fn description(contenu: &Contenu<'_>) -> String {
    let badges = joindre(&[
        if contenu.playset {
            Sorte::Playset.libelle()
        } else {
            ""
        },
        contenu.prix.unwrap_or_default(),
        if contenu.overframe {
            Sorte::Overframe.libelle()
        } else {
            ""
        },
    ]);
    let numero = numero_et_rarete(contenu.set_code, contenu.rarete);
    [contenu.nom.trim(), numero.as_str(), badges.as_str()]
        .into_iter()
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    fn carte(largeur: f32, hauteur: f32) -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(largeur, hauteur))
    }

    fn proche(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    /// La carte habituelle : tout à sa place d'origine, en entier — PLAYSET
    /// à gauche, le prix au centre, OVERFRAME à droite, sur la ligne du bas.
    #[test]
    fn la_ligne_du_bas_garde_les_places_d_origine() {
        let c = carte(260.0, 380.0);
        let d = disposer(c, Some(44.0), true, true);
        let playset = d.playset.unwrap();
        let overframe = d.overframe.unwrap();
        let prix = d.prix.unwrap();
        assert_eq!(playset.taille, Taille::Entiere);
        assert_eq!(overframe.taille, Taille::Entiere);
        assert!(proche(playset.zone.left(), RETRAIT_BADGE));
        assert!(proche(overframe.zone.right(), 260.0 - RETRAIT_BADGE));
        assert!(proche(prix.center().x, 130.0));
        for zone in [playset.zone, prix, overframe.zone] {
            assert!(proche(zone.bottom(), 380.0 - RETRAIT_BADGE));
            assert!(proche(zone.height(), HAUTEUR_BADGE));
        }
    }

    /// Étroite : le prix garde le centre, les deux badges passent ensemble à
    /// leur sigle, puis à leur couleur.
    #[test]
    fn sur_une_carte_etroite_les_badges_cedent_au_prix() {
        let tailles = |d: Disposition| (d.playset.map(|b| b.taille), d.overframe.map(|b| b.taille));
        let c = carte(120.0, 175.0);
        let d = disposer(c, Some(44.0), true, true);
        assert!(proche(d.prix.unwrap().center().x, 60.0));
        assert_eq!(tailles(d), (Some(Taille::Sigle), Some(Taille::Sigle)));

        // La plus petite carte permise : il ne reste que les couleurs.
        let d = disposer(carte(90.0, 131.0), Some(42.0), true, true);
        assert!(proche(d.prix.unwrap().center().x, 45.0));
        assert_eq!(tailles(d), (Some(Taille::Pastille), Some(Taille::Pastille)));

        // Un seul badge, sans prix : le mot entier tient.
        let d = disposer(c, None, false, true);
        assert_eq!(tailles(d), (None, Some(Taille::Entiere)));

        // Les deux sans prix : ils se touchent en entier, donc réduits tous
        // les deux — jamais un mot entier d'un côté et un sigle de l'autre.
        let d = disposer(carte(110.0, 160.0), None, true, true);
        assert_eq!(tailles(d), (Some(Taille::Sigle), Some(Taille::Sigle)));
    }

    /// Sans même la place d'une pastille, le badge s'efface ; le prix reste.
    /// Plus étroite que le prix lui-même, la carte ne le montre pas.
    #[test]
    fn le_prix_passe_avant_les_badges() {
        let d = disposer(carte(40.0, 58.0), Some(30.0), true, true);
        assert!(d.prix.is_some());
        assert_eq!(d.playset, None);
        assert_eq!(d.overframe, None);

        let d = disposer(carte(50.0, 73.0), Some(60.0), false, true);
        assert_eq!(d.prix, None);
        assert!(d.overframe.is_some());
    }

    /// Au repos, le bandeau tient dans la boîte de texte ; ses formes vont
    /// de la plus complète à rien, selon la hauteur de la carte.
    #[test]
    fn la_forme_au_repos_suit_la_hauteur_de_la_carte() {
        assert!(Forme::Complete.hauteur() > Forme::Compacte.hauteur());
        assert!(proche(Forme::Pastilles.hauteur(), 0.0));
        for hauteur in [131.0, 141.0, 160.0, 193.0, 380.0, 1185.0] {
            let forme = Forme::au_repos(hauteur);
            assert!(
                forme.hauteur() <= hauteur * PART_MAX,
                "{hauteur} → {forme:?}"
            );
        }
        assert_eq!(Forme::au_repos(193.0), Forme::Complete);
        assert_eq!(Forme::au_repos(192.0), Forme::Compacte);
        assert_eq!(Forme::au_repos(141.0), Forme::Compacte);
        assert_eq!(Forme::au_repos(140.0), Forme::Pastilles);
    }

    /// Dessine sans fenêtre et rend ce que chaque appel a renvoyé.
    fn rendus(cas: &[(egui::Rect, Contenu<'_>)]) -> Vec<Option<String>> {
        let ctx = egui::Context::default();
        let mut rendus = Vec::new();
        let mut sortie = ctx.run_ui(egui::RawInput::default(), |ui| {
            rendus.clear();
            for (c, contenu) in cas {
                rendus.push(dessiner(ui.painter(), *c, contenu, Forme::Complete, false));
            }
        });
        // La première image remplit l'atlas des polices : sans moteur de rendu
        // pour l'appliquer, on le vide avant de jeter la sortie.
        sortie.textures_delta.clear();
        rendus
    }

    /// L'infobulle ne vient que si quelque chose a été raccourci, et dit
    /// alors tout.
    #[test]
    fn ce_qui_est_raccourci_se_lit_dans_l_infobulle() {
        let chaos = Contenu {
            nom: "Chaos Sombre",
            set_code: "CORI-JP001",
            rarete: "Super Rare",
            playset: false,
            overframe: false,
            prix: Some("3,88 €"),
            possedee: true,
        };
        let long = Contenu {
            nom: "Un nom de carte bien trop long pour tenir sur la ligne du bandeau",
            ..chaos
        };
        let badges = Contenu {
            playset: true,
            overframe: true,
            ..chaos
        };
        let r = rendus(&[
            (carte(260.0, 380.0), chaos),
            (carte(260.0, 380.0), long),
            (carte(110.0, 160.0), badges),
        ]);
        assert_eq!(r.len(), 3);
        assert_eq!(r[0], None, "tout tient en entier");
        assert!(
            r[1].as_deref()
                .is_some_and(|t| t.starts_with(long.nom) && t.contains("3,88 €")),
            "{:?}",
            r[1]
        );
        assert_eq!(
            r[2].as_deref(),
            Some("Chaos Sombre\nCORI-JP001 · Super Rare\nPLAYSET · 3,88 € · OVERFRAME")
        );
    }
}
