// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Construction des URL d'images de cartes.
//!
//! Portage **pur** de `module/config_image_source.py`. La partie « préférence
//! stockée » vit dans [`crate::config::SourceImage`] ; la partie « recherche
//! d'une URL de repli dans `cardinfo.db` » vit dans `ygo-db` (règle R1).
//!
//! # L'invariant central : un identifiant négatif signale un artwork externe
//!
//! Les artworks récupérés sur Yugipedia — variantes d'illustration absentes de
//! l'API YGOPRODeck — n'ont pas de « password » Konami. On leur attribue un
//! identifiant **synthétique négatif** : le CRC32 du nom de fichier, interprété
//! comme entier signé. Les identifiants YGOPRODeck étant toujours positifs, le
//! **signe suffit** à les distinguer.
//!
//! Conséquence, à ne perdre sous aucun prétexte : pour ces artworks, l'URL
//! n'est **jamais** reconstruite depuis l'identifiant ; elle est lue telle
//! quelle dans `card_image_url`. Toute la chaîne d'affichage en dépend, ainsi
//! que le calcul des rangs Art A / Art B dans le tri.
//!
//! Exemple relevé sur EGO1 dans `cardinfo.db` :
//!
//! ```text
//! BrainControl-EGO1-EN-C-1E.png  → -454078694
//! SoulCrossing-EGO1-EN-UR-UE.png → -1878813272
//! ```

use crate::config::SourceImage;

/// Modèle d'URL YGOPRODeck — le suffixe est le « password » Konami.
pub const YGOPRODECK_IMG_BASE: &str = "https://images.ygoprodeck.com/images/cards/";

/// Un `card_image_id` désigne-t-il un artwork **externe** (Yugipedia) ?
///
/// Portage de `config_image_source.est_image_externe()`.
///
/// ```
/// use ygo_core::image_source::est_image_externe;
/// assert!(est_image_externe(Some(-454078694)));
/// assert!(!est_image_externe(Some(101206062)));
/// assert!(!est_image_externe(Some(0)));
/// assert!(!est_image_externe(None));
/// ```
pub fn est_image_externe(card_image_id: Option<i64>) -> bool {
    card_image_id.is_some_and(|id| id < 0)
}

/// URL YGOPRODeck construite depuis l'identifiant.
pub fn url_ygoprodeck(card_image_id: i64) -> String {
    format!("{YGOPRODECK_IMG_BASE}{card_image_id}.jpg")
}

/// URL à télécharger pour une carte, selon la source configurée.
///
/// Portage de `config_image_source.build_image_url()`.
///
/// - Artwork **externe** : l'URL stockée, quelle que soit la source active.
/// - `YGOPRODECK` : URL construite depuis l'identifiant ; repli sur l'URL
///   stockée si l'identifiant est absent **ou nul** (le Python teste
///   `if card_image_id:`, donc `0` est traité comme absent).
/// - `YUGIPEDIA` : l'URL stockée telle quelle.
///
/// Retourne `None` quand aucune URL n'est exploitable (chaîne vide comprise).
pub fn url_image(
    source: SourceImage,
    card_image_url: Option<&str>,
    card_image_id: Option<i64>,
) -> Option<String> {
    let stockee = card_image_url.filter(|u| !u.is_empty()).map(str::to_owned);

    if est_image_externe(card_image_id) {
        return stockee;
    }

    match source {
        SourceImage::Ygoprodeck => match card_image_id {
            Some(id) if id != 0 => Some(url_ygoprodeck(id)),
            _ => stockee,
        },
        SourceImage::Yugipedia => stockee,
    }
}

/// URL de la source **alternative**, utilisée par le téléchargeur quand la
/// première a échoué.
///
/// Portage de `config_image_source.build_fallback_url()`, moins la recherche en
/// base :
///
/// - Artwork externe : `None` — il n'existe qu'une seule URL.
/// - Source active `YUGIPEDIA` : l'URL YGOPRODeck construite depuis
///   l'identifiant.
/// - Source active `YGOPRODECK` : le repli est une URL Yugipedia qu'il faut
///   chercher dans `cardinfo.db` (`card_images.card_url` via
///   `ygoprodeck_image_id`). Cette recherche relevant de SQLite, elle est
///   passée en paramètre — `ygo-db` fournit la fermeture en production, les
///   tests en fournissent une factice.
pub fn url_repli<F>(
    source: SourceImage,
    card_image_id: Option<i64>,
    chercher_url_yugipedia: F,
) -> Option<String>
where
    F: FnOnce(i64) -> Option<String>,
{
    if est_image_externe(card_image_id) {
        return None;
    }
    let id = card_image_id.filter(|id| *id != 0)?;

    match source {
        SourceImage::Ygoprodeck => chercher_url_yugipedia(id),
        SourceImage::Yugipedia => Some(url_ygoprodeck(id)),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    const URL_YUGI: &str = "https://ms.yugipedia.com//0/01/BrainControl-EGO1-EN-C-1E.png";

    #[test]
    fn identifiant_negatif_signale_un_artwork_externe() {
        assert!(est_image_externe(Some(-454078694)));
        assert!(est_image_externe(Some(-1)));
        assert!(!est_image_externe(Some(0)));
        assert!(!est_image_externe(Some(101206062)));
        assert!(!est_image_externe(None));
    }

    #[test]
    fn artwork_externe_sert_toujours_l_url_stockee() {
        // Même quand la source configurée est YGOPRODECK : l'identifiant est
        // synthétique, il ne désigne aucune image chez YGOPRODeck.
        for source in [SourceImage::Ygoprodeck, SourceImage::Yugipedia] {
            assert_eq!(
                url_image(source, Some(URL_YUGI), Some(-454078694)).as_deref(),
                Some(URL_YUGI)
            );
            assert_eq!(
                url_repli(source, Some(-454078694), |_| Some("x".into())),
                None
            );
        }
    }

    #[test]
    fn ygoprodeck_construit_l_url_depuis_l_identifiant() {
        assert_eq!(
            url_image(SourceImage::Ygoprodeck, Some(URL_YUGI), Some(101206062)).as_deref(),
            Some("https://images.ygoprodeck.com/images/cards/101206062.jpg")
        );
    }

    #[test]
    fn ygoprodeck_sans_identifiant_replie_sur_l_url_stockee() {
        assert_eq!(
            url_image(SourceImage::Ygoprodeck, Some(URL_YUGI), None).as_deref(),
            Some(URL_YUGI)
        );
        // `0` est traité comme absent, comme le `if card_image_id:` du Python.
        assert_eq!(
            url_image(SourceImage::Ygoprodeck, Some(URL_YUGI), Some(0)).as_deref(),
            Some(URL_YUGI)
        );
    }

    #[test]
    fn yugipedia_sert_l_url_stockee() {
        assert_eq!(
            url_image(SourceImage::Yugipedia, Some(URL_YUGI), Some(101206062)).as_deref(),
            Some(URL_YUGI)
        );
    }

    #[test]
    fn aucune_url_exploitable() {
        assert_eq!(url_image(SourceImage::Yugipedia, None, Some(123)), None);
        assert_eq!(url_image(SourceImage::Yugipedia, Some(""), Some(123)), None);
        assert_eq!(url_image(SourceImage::Ygoprodeck, None, None), None);
    }

    #[test]
    fn repli_selon_la_source_active() {
        // Source YUGIPEDIA → repli YGOPRODeck construit depuis l'identifiant.
        assert_eq!(
            url_repli(SourceImage::Yugipedia, Some(101206062), |_| None).as_deref(),
            Some("https://images.ygoprodeck.com/images/cards/101206062.jpg")
        );
        // Source YGOPRODECK → repli Yugipedia, cherché en base.
        assert_eq!(
            url_repli(SourceImage::Ygoprodeck, Some(101206062), |id| {
                assert_eq!(id, 101206062);
                Some(URL_YUGI.to_owned())
            })
            .as_deref(),
            Some(URL_YUGI)
        );
        // Aucune correspondance en base → pas de repli.
        assert_eq!(url_repli(SourceImage::Ygoprodeck, Some(1), |_| None), None);
        // Pas d'identifiant → pas de repli.
        assert_eq!(
            url_repli(SourceImage::Ygoprodeck, None, |_| Some("x".into())),
            None
        );
    }
}
