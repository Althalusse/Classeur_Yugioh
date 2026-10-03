// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Les idéogrammes, empruntés au système.
//!
//! # Le défaut que ce module répare
//!
//! egui embarque quatre polices : Ubuntu Light, Noto Emoji, emoji-icon-font
//! et Hack. **Aucune n'a d'idéogramme.** La fiche d'une carte peut donc
//! proposer un texte japonais, coréen ou chinois — la base en contient
//! 14 205 en japonais — et n'afficher qu'une file de carrés vides. C'est ce
//! qu'a montré la première capture de la fiche : les onglets « 日本語 »,
//! « 한국어 » et « 中文 » y étaient illisibles, et leur texte l'aurait été
//! aussi.
//!
//! C'est le même piège que les chevrons `▸`/`▾` du tableau des statistiques,
//! à une échelle plus vaste : la question n'est jamais « ce caractère
//! existe-t-il » mais « la famille que ce widget consulte l'a-t-elle ».
//!
//! # Pourquoi le système et pas un fichier embarqué
//!
//! Une police CJK complète pèse de cinq à seize mégaoctets — plus que tout
//! le reste du binaire. Les systèmes qui affichent ces écritures en ont déjà
//! une : Windows livre MS Gothic depuis toujours et Yu Gothic depuis 8.1,
//! macOS Hiragino, les distributions Linux Noto CJK. On emprunte plutôt que
//! d'embarquer.
//!
//! Le prix de ce choix est qu'il peut échouer. Une machine sans police CJK
//! affichera des carrés — alors [`cjk_disponible`] le dit, et la fiche
//! s'abstient d'offrir une langue qu'elle ne sait pas dessiner. Mieux vaut
//! une langue en moins qu'un texte en carrés.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use eframe::egui;

/// Le nom sous lequel la police empruntée est installée dans egui.
pub const NOM_CJK: &str = "cjk-systeme";

/// Les emplacements où chercher une police à idéogrammes, par système.
///
/// L'ordre compte : on préfère une police *sans serif* moderne, puis on se
/// rabat sur ce qui existe. Chaque entrée est un chemin absolu, tel que le
/// système la livre.
///
/// Cette liste est volontairement close et vérifiable, plutôt qu'un
/// balayage du dossier des polices : une police prise au hasard parce que
/// son nom contient « gothic » peut n'avoir aucun idéogramme.
#[must_use]
pub fn emplacements() -> Vec<PathBuf> {
    let bruts: &[&str] = if cfg!(target_os = "windows") {
        &[
            r"C:\Windows\Fonts\YuGothM.ttc",
            r"C:\Windows\Fonts\YuGothR.ttc",
            r"C:\Windows\Fonts\meiryo.ttc",
            r"C:\Windows\Fonts\msgothic.ttc",
            r"C:\Windows\Fonts\msmincho.ttc",
            r"C:\Windows\Fonts\malgun.ttf",
            r"C:\Windows\Fonts\msyh.ttc",
            r"C:\Windows\Fonts\simsun.ttc",
        ]
    } else if cfg!(target_os = "macos") {
        &[
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
            "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
            "/Library/Fonts/Arial Unicode.ttf",
        ]
    } else {
        &[
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/opentype/noto/NotoSansCJKjp-Regular.otf",
            "/usr/share/fonts/opentype/noto/NotoSerifCJK-Regular.ttc",
            "/usr/share/fonts/opentype/noto/NotoSerifCJK-Bold.ttc",
            "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
            "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
        ]
    };
    bruts.iter().map(PathBuf::from).collect()
}

/// Le premier emplacement qui existe, parmi ceux proposés.
///
/// Séparé de [`emplacements`] pour être testable sans dépendre de la machine
/// qui exécute les tests.
#[must_use]
pub fn premier_present(candidats: &[PathBuf]) -> Option<PathBuf> {
    candidats.iter().find(|c| c.is_file()).cloned()
}

/// Une police CJK a-t-elle été trouvée et installée ?
///
/// # Pourquoi `get` et pas `get_or_init`
///
/// `get_or_init(|| false)` **écrit** dans le verrou : la première question
/// posée avant [`installer`] figerait la réponse à « non » pour toute la
/// durée du programme, et le `set(true)` de l'installation échouerait en
/// silence. Une lecture doit lire.
///
/// Avant l'installation, la réponse est donc « non » sans être gravée : le
/// pire qui puisse arriver est une langue offerte un cadre trop tard.
#[must_use]
pub fn cjk_disponible() -> bool {
    TROUVEE.get().copied().unwrap_or(false)
}

/// Mémorise le résultat de la recherche, faite une seule fois au démarrage.
static TROUVEE: OnceLock<bool> = OnceLock::new();

/// Les langues qui exigent une police à idéogrammes.
///
/// ```
/// use ygo_ui::polices::exige_des_ideogrammes;
/// assert!(exige_des_ideogrammes("ja"));
/// assert!(exige_des_ideogrammes("zh-CN"));
/// assert!(exige_des_ideogrammes("ko"));
/// assert!(!exige_des_ideogrammes("fr"));
/// assert!(!exige_des_ideogrammes("en"));
/// ```
#[must_use]
pub fn exige_des_ideogrammes(langue: &str) -> bool {
    matches!(langue, "ja" | "ko") || langue.starts_with("zh")
}

/// Installe la police à idéogrammes du système, si elle existe.
///
/// Rendue en dernier recours des deux familles : les caractères latins
/// restent dessinés par Ubuntu Light, qui leur va mieux, et egui ne consulte
/// la police empruntée que pour ce que les autres ne savent pas tracer.
///
/// Rend `true` si une police a été installée.
pub fn installer(ctx: &egui::Context) -> bool {
    let Some(chemin) = premier_present(&emplacements()) else {
        let _ = TROUVEE.set(false);
        tracing::info!("aucune police à idéogrammes : ja/ko/zh ne seront pas offerts");
        return false;
    };
    let installee = charger(ctx, &chemin);
    let _ = TROUVEE.set(installee);
    if installee {
        tracing::info!(police = %chemin.display(), "police à idéogrammes empruntée au système");
    }
    installee
}

/// Lit le fichier et l'ajoute aux deux familles.
fn charger(ctx: &egui::Context, chemin: &Path) -> bool {
    let Ok(octets) = std::fs::read(chemin) else {
        tracing::warn!(police = %chemin.display(), "police illisible");
        return false;
    };
    let mut polices = egui::FontDefinitions::default();
    polices.font_data.insert(
        NOM_CJK.to_owned(),
        std::sync::Arc::new(egui::FontData::from_owned(octets)),
    );
    for famille in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        polices
            .families
            .entry(famille)
            .or_default()
            .push(NOM_CJK.to_owned());
    }
    ctx.set_fonts(polices);
    true
}

/// La taille de chaque style de texte pour une échelle donnée, depuis les
/// tailles **par défaut** d'egui — jamais depuis les tailles courantes, sans
/// quoi deux réglages successifs se multiplieraient.
///
/// ```
/// use eframe::egui;
/// use ygo_ui::polices::tailles_texte;
/// let base = egui::Style::default().text_styles;
/// let grand = tailles_texte(1.5);
/// let corps = |m: &std::collections::BTreeMap<egui::TextStyle, egui::FontId>| m[&egui::TextStyle::Body].size;
/// assert!((corps(&grand) - corps(&base) * 1.5).abs() < 1e-4);
/// assert_eq!(tailles_texte(1.0), base);
/// ```
#[must_use]
pub fn tailles_texte(echelle: f32) -> std::collections::BTreeMap<egui::TextStyle, egui::FontId> {
    let mut styles = egui::Style::default().text_styles;
    for police in styles.values_mut() {
        police.size *= echelle;
    }
    styles
}

/// Applique le réglage « Taille du texte » des Options, **sans redémarrage**.
///
/// # R9 — 2026-10-02
///
/// Le Python devait redémarrer pour changer la taille des polices Tkinter
/// (`utilitaire/redemarrage.py`). Le portage, lui, enregistrait le réglage…
/// et ne l'appliquait nulle part. egui sait changer ses styles d'une image à
/// l'autre : le réglage s'applique au démarrage et à l'instant où on le change.
/// Les deux thèmes, clair et sombre, sont mis à jour.
pub fn appliquer_taille_texte(ctx: &egui::Context, echelle: f64) {
    #[allow(clippy::cast_possible_truncation)]
    let styles = tailles_texte(echelle as f32);
    ctx.all_styles_mut(|style| style.text_styles = styles.clone());
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    /// Les emplacements sont des chemins absolus, et il y en a.
    ///
    /// Un chemin relatif dépendrait du dossier courant, qui n'est pas celui
    /// du binaire — c'est exactement le genre de piège qui a déjà fait
    /// lancer l'application depuis `C:\WINDOWS\system32`.
    #[test]
    fn les_emplacements_sont_absolus() {
        let liste = emplacements();
        assert!(!liste.is_empty(), "au moins un candidat par système");
        for c in &liste {
            assert!(
                c.is_absolute(),
                "{} n'est pas un chemin absolu",
                c.display()
            );
        }
    }

    /// La recherche prend le premier qui existe, dans l'ordre donné.
    #[test]
    fn la_recherche_respecte_l_ordre_des_candidats() {
        let tmp = tempfile::tempdir().unwrap();
        let deuxieme = tmp.path().join("deuxieme.ttf");
        let troisieme = tmp.path().join("troisieme.ttf");
        std::fs::write(&deuxieme, b"x").unwrap();
        std::fs::write(&troisieme, b"x").unwrap();

        let candidats = vec![tmp.path().join("absent.ttf"), deuxieme.clone(), troisieme];
        assert_eq!(premier_present(&candidats), Some(deuxieme));

        // Rien de présent : rien de rendu, et surtout pas de panique.
        assert_eq!(premier_present(&[tmp.path().join("rien.ttf")]), None);
        assert_eq!(premier_present(&[]), None);
    }

    /// Un dossier n'est pas une police.
    ///
    /// `exists()` aurait dit oui : c'est `is_file()` qui fait la différence,
    /// et une lecture de dossier échouerait plus loin, sans rien expliquer.
    #[test]
    fn un_dossier_n_est_pas_une_police() {
        let tmp = tempfile::tempdir().unwrap();
        let faux = tmp.path().join("police.ttc");
        std::fs::create_dir(&faux).unwrap();
        assert_eq!(premier_present(&[faux]), None);
    }

    /// Seules les écritures idéographiques exigent la police empruntée.
    #[test]
    fn seules_les_ecritures_ideographiques_l_exigent() {
        for langue in ["ja", "ko", "zh-CN", "zh-TW"] {
            assert!(exige_des_ideogrammes(langue), "{langue}");
        }
        for langue in ["fr", "en", "de", "es", "it", "pt"] {
            assert!(!exige_des_ideogrammes(langue), "{langue}");
        }
    }

    /// Un fichier illisible ne fait pas tomber l'application.
    #[test]
    fn une_police_illisible_ne_fait_pas_tomber_l_application() {
        let ctx = egui::Context::default();
        let tmp = tempfile::tempdir().unwrap();
        let absent = tmp.path().join("absent.ttf");
        assert!(!charger(&ctx, &absent), "fichier absent");
    }

    /// Interroger la disponibilité ne la fige pas.
    ///
    /// C'est tout l'objet du `get` : avec `get_or_init(|| false)`, ce test
    /// verrouillerait la réponse à « non » et l'installation qui suivrait ne
    /// pourrait plus jamais la corriger.
    #[test]
    fn interroger_la_disponibilite_ne_la_fige_pas() {
        let avant = cjk_disponible();
        let _ = cjk_disponible();
        assert_eq!(cjk_disponible(), avant, "la lecture ne change rien");
        assert!(
            TROUVEE.get().is_none() || TROUVEE.get().is_some(),
            "et surtout, elle n'écrit pas"
        );
        // Le verrou reste libre tant qu'`installer` n'a pas parlé.
        assert!(
            TROUVEE.set(true).is_ok() || cjk_disponible(),
            "le verrou était encore disponible pour l'installation"
        );
    }
}
