// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! L'atténuation des cartes non possédées.
//!
//! Portage de `_load_card_image` — la seule part de l'affichage qui se laisse
//! éprouver, parce que c'est une transformation **pure** de pixels et non un
//! dessin.
//!
//! # Ce que fait le Python
//!
//! Une carte que l'utilisateur ne possède pas est délavée, pour qu'un coup
//! d'œil sur une page suffise à voir ce qui manque :
//!
//! | état | couleur | luminosité |
//! |---|---|---|
//! | possédée | — | — |
//! | non possédée | **0,20** | **0,35** |
//! | non possédée, survolée | **0,70** | **0,70** |
//!
//! Le survol rend donc la carte presque lisible sans la rendre franche : on
//! peut l'examiner sans la confondre avec une carte de la collection.

/// Facteurs appliqués à une carte selon son état.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Facteurs {
    /// Part de couleur conservée — `1.0` laisse l'image intacte, `0.0` la
    /// rend en niveaux de gris.
    pub couleur: f32,
    /// Part de luminosité conservée — `1.0` intacte, `0.0` noire.
    pub luminosite: f32,
}

impl Facteurs {
    /// Aucune atténuation.
    pub const INTACTE: Self = Self {
        couleur: 1.0,
        luminosite: 1.0,
    };
    /// Carte non possédée, au repos.
    pub const ABSENTE: Self = Self {
        couleur: 0.20,
        luminosite: 0.35,
    };
    /// Carte non possédée, survolée.
    pub const ABSENTE_SURVOLEE: Self = Self {
        couleur: 0.70,
        luminosite: 0.70,
    };

    /// Les facteurs qui conviennent à un état.
    #[must_use]
    pub fn pour(possedee: bool, survolee: bool) -> Self {
        match (possedee, survolee) {
            (true, _) => Self::INTACTE,
            (false, true) => Self::ABSENTE_SURVOLEE,
            (false, false) => Self::ABSENTE,
        }
    }

    /// N'y a-t-il rien à faire ?
    #[must_use]
    pub fn sans_effet(self) -> bool {
        (self.couleur - 1.0).abs() < f32::EPSILON && (self.luminosite - 1.0).abs() < f32::EPSILON
    }
}

/// Luminance perçue, telle que Pillow la calcule pour convertir en `L`.
///
/// Ce sont les coefficients ITU-R BT.601 employés par `Image.convert("L")` —
/// et donc par `ImageEnhance.Color`, qui mélange l'image avec sa version en
/// niveaux de gris.
#[must_use]
pub fn luminance(r: u8, v: u8, b: u8) -> f32 {
    0.299 * f32::from(r) + 0.587 * f32::from(v) + 0.114 * f32::from(b)
}

/// Atténue une image RGBA **en place**.
///
/// Portage de l'enchaînement `ImageEnhance.Color` puis
/// `ImageEnhance.Brightness` :
///
/// 1. **couleur** — mélange linéaire entre le gris et l'original :
///    `sortie = gris + f × (original − gris)` ;
/// 2. **luminosité** — mélange avec le noir : `sortie = f × entrée`.
///
/// # L'ordre ne compte pas, et il vaut mieux le savoir
///
/// On croirait que désaturer puis assombrir diffère d'assombrir puis
/// désaturer. **Les deux donnent exactement le même résultat**, et la
/// démonstration tient en une ligne : la désaturation est
/// `D(x) = g + f·(x − g)` avec `g = w·x`, donc **linéaire** en `x` ; la
/// luminosité est une multiplication scalaire. Or `g(bx) = b·g(x)`, d'où
/// `D(bx) = b·D(x)`. Les deux opérations commutent.
///
/// Ce paragraphe existe parce que le commentaire disait d'abord l'inverse, et
/// que le test écrit pour le prouver a démontré le contraire. Sans lui,
/// quelqu'un « corrigera » un jour un ordre qui n'a jamais eu d'importance.
///
/// Le canal alpha n'est pas touché : une carte absente reste opaque, elle est
/// délavée, pas transparente.
pub fn attenuer(pixels: &mut [u8], facteurs: Facteurs) {
    if facteurs.sans_effet() {
        return;
    }
    for pixel in pixels.chunks_exact_mut(4) {
        let (Some(&r), Some(&v), Some(&b)) = (pixel.first(), pixel.get(1), pixel.get(2)) else {
            continue;
        };
        let gris = luminance(r, v, b);

        let melange = |canal: u8| -> u8 {
            let desature = gris + facteurs.couleur * (f32::from(canal) - gris);
            let assombri = desature * facteurs.luminosite;
            // Pillow arrondit au plus proche puis borne à [0, 255].
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            {
                assombri.round().clamp(0.0, 255.0) as u8
            }
        };

        let (nr, nv, nb) = (melange(r), melange(v), melange(b));
        if let Some(canal) = pixel.first_mut() {
            *canal = nr;
        }
        if let Some(canal) = pixel.get_mut(1) {
            *canal = nv;
        }
        if let Some(canal) = pixel.get_mut(2) {
            *canal = nb;
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    fn pixel(r: u8, v: u8, b: u8, facteurs: Facteurs) -> (u8, u8, u8, u8) {
        let mut px = [r, v, b, 255];
        attenuer(&mut px, facteurs);
        (px[0], px[1], px[2], px[3])
    }

    #[test]
    fn une_carte_possedee_n_est_pas_touchee() {
        assert_eq!(pixel(200, 40, 10, Facteurs::INTACTE), (200, 40, 10, 255));
    }

    #[test]
    fn l_alpha_ne_change_jamais() {
        // Une carte absente est délavée, pas transparente.
        let mut px = [200, 40, 10, 128];
        attenuer(&mut px, Facteurs::ABSENTE);
        assert_eq!(px[3], 128);
    }

    #[test]
    fn une_carte_absente_est_desaturee_puis_assombrie() {
        // Rouge vif : luminance = 0.299 × 255 = 76.245
        // couleur 0.20  → 76.245 + 0.20 × (255 − 76.245) = 111.996
        // luminosité 0.35 → 39.2 → 39
        let (r, v, b, _) = pixel(255, 0, 0, Facteurs::ABSENTE);
        assert_eq!(r, 39);
        // vert : 76.245 + 0.20 × (0 − 76.245) = 60.996 ; × 0.35 = 21.3 → 21
        assert_eq!(v, 21);
        assert_eq!(b, 21, "bleu et vert partent tous deux de zéro");
    }

    #[test]
    fn le_survol_eclaircit_sans_rendre_la_carte_franche() {
        let repos = pixel(255, 0, 0, Facteurs::ABSENTE);
        let survol = pixel(255, 0, 0, Facteurs::ABSENTE_SURVOLEE);
        assert!(survol.0 > repos.0, "plus lumineuse au survol");
        assert!(
            survol.0 < 255,
            "mais toujours pas au niveau d'une carte possédée"
        );
    }

    #[test]
    fn desaturer_et_assombrir_commutent() {
        // Contre-intuitif, et démontré plutôt que supposé : la désaturation
        // est linéaire, la luminosité est une multiplication scalaire, donc
        // l'ordre est sans effet. Ce test fige le fait pour qu'on ne
        // « corrige » pas un jour un ordre qui n'a jamais compté.
        let f = Facteurs::ABSENTE;
        for (r, v, b) in [(255, 0, 0), (12, 200, 90), (255, 255, 255), (0, 0, 0)] {
            let (dans_l_ordre, _, _, _) = pixel(r, v, b, f);

            // L'ordre inverse, calculé à la main sur le canal rouge.
            let (sr, sv, sb) = (
                f32::from(r) * f.luminosite,
                f32::from(v) * f.luminosite,
                f32::from(b) * f.luminosite,
            );
            let gris = 0.299 * sr + 0.587 * sv + 0.114 * sb;
            let inverse = (gris + f.couleur * (sr - gris)).round();

            assert!(
                (f32::from(dans_l_ordre) - inverse).abs() <= 1.0,
                "({r},{v},{b}) : {dans_l_ordre} contre {inverse} — \
                 les deux ordres doivent coïncider"
            );
        }
    }

    #[test]
    fn un_gris_reste_gris_quelle_que_soit_la_desaturation() {
        // Sa luminance vaut déjà sa valeur : la désaturation ne peut rien y
        // changer, seule la luminosité agit.
        let (r, v, b, _) = pixel(100, 100, 100, Facteurs::ABSENTE);
        assert_eq!((r, v, b), (35, 35, 35), "100 × 0.35 = 35");
    }

    #[test]
    fn le_noir_et_le_blanc_restent_dans_les_bornes() {
        assert_eq!(pixel(0, 0, 0, Facteurs::ABSENTE), (0, 0, 0, 255));
        let (r, _, _, _) = pixel(255, 255, 255, Facteurs::ABSENTE);
        assert_eq!(r, 89, "255 × 0.35 = 89.25 → 89, jamais au-delà de 255");
    }

    #[test]
    fn les_facteurs_suivent_l_etat_de_la_carte() {
        assert_eq!(Facteurs::pour(true, false), Facteurs::INTACTE);
        assert_eq!(
            Facteurs::pour(true, true),
            Facteurs::INTACTE,
            "survoler une carte possédée ne l'altère pas"
        );
        assert_eq!(Facteurs::pour(false, false), Facteurs::ABSENTE);
        assert_eq!(Facteurs::pour(false, true), Facteurs::ABSENTE_SURVOLEE);
    }

    #[test]
    fn une_image_intacte_n_est_meme_pas_parcourue() {
        let mut pixels = vec![7_u8; 4 * 1000];
        attenuer(&mut pixels, Facteurs::INTACTE);
        assert!(pixels.iter().all(|&o| o == 7));
    }

    #[test]
    fn la_luminance_est_celle_de_pillow() {
        // Coefficients ITU-R BT.601, ceux d'`Image.convert("L")`.
        assert!((luminance(255, 0, 0) - 76.245).abs() < 0.001);
        assert!((luminance(0, 255, 0) - 149.685).abs() < 0.001);
        assert!((luminance(0, 0, 255) - 29.07).abs() < 0.001);
        assert!((luminance(255, 255, 255) - 255.0).abs() < 0.001);
    }
}
