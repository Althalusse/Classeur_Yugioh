// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Ce que le client HTTP garde sur le disque pour ne pas redemander.
//!
//! # Pourquoi — 2026-10-01
//!
//! L'application sera partagée. Ses règles d'accès aux sources doivent donc
//! tenir pour cent utilisateurs comme pour un, et elles sont écrites par les
//! sources elles-mêmes :
//!
//! - **Yugipedia** (`Yugipedia:API`) : « Results of requests should be cached
//!   wherever possible […] A good period for caching is 30 days. » ;
//! - **YGOPRODeck** (guide de l'API) : « Please download and store all data
//!   pulled from this API locally to keep the amount of API calls used to a
//!   minimum », et une image se télécharge **une fois**.
//!
//! Avant ce module, l'écran des artworks relisait la Set list de chaque set à
//! chaque ouverture, une création de classeur refaisait toutes ses recherches
//! Yugipedia, et une image ou une couverture introuvable (404) était
//! redemandée à **chaque** ouverture du classeur, sans fin.
//!
//! # Deux mémoires
//!
//! 1. **Les réponses d'API**, par URL, pour une durée fixée par
//!    [`duree_de_cache`] : 30 jours chez Yugipedia, 2 jours pour les listes de
//!    sets d'YGOPRODeck — la durée de leur propre cache serveur. Le contrôle de
//!    version et le catalogue complet n'y passent **jamais** : le premier dit
//!    s'il faut mettre à jour, le second **est** la mise à jour.
//! 2. **Les adresses introuvables** (404, 410), pour [`DUREE_ECHEC`] : une
//!    image absente n'est redemandée qu'au bout de sept jours. Une panne
//!    réseau, elle, n'est **pas** retenue — sinon une soirée hors ligne
//!    priverait d'images pendant une semaine.
//!
//! Tout vit dans `bdd/cache_http/`. Le supprimer remet tout à zéro, sans
//! autre conséquence qu'un peu de réseau.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Durée de vie d'une réponse d'API Yugipedia — la période que le wiki conseille.
pub const DUREE_YUGIPEDIA: Duration = Duration::from_secs(30 * 24 * 3600);

/// Durée de vie d'une liste de sets ou de cartes d'un set chez YGOPRODeck —
/// celle de leur cache serveur (« cached for 2 days »).
pub const DUREE_YGOPRODECK: Duration = Duration::from_secs(2 * 24 * 3600);

/// Combien de temps une adresse introuvable n'est pas redemandée.
pub const DUREE_ECHEC: Duration = Duration::from_secs(7 * 24 * 3600);

/// Statuts qui disent « cette ressource n'existe pas », et rien d'autre.
pub const STATUTS_INTROUVABLES: [u16; 2] = [404, 410];

/// Combien de temps garder la réponse de cette URL — `None` : jamais.
///
/// Fonction **pure**.
///
/// ```
/// use ygo_sources::cache::{duree_de_cache, DUREE_YGOPRODECK, DUREE_YUGIPEDIA};
/// let api = "https://yugipedia.com/api.php?action=parse&page=X";
/// assert_eq!(duree_de_cache(api), Some(DUREE_YUGIPEDIA));
/// let sets = "https://db.ygoprodeck.com/api/v7/cardsets.php";
/// assert_eq!(duree_de_cache(sets), Some(DUREE_YGOPRODECK));
/// let set = "https://db.ygoprodeck.com/api/v7/cardinfo.php?cardset=Rarity%20Collection%205";
/// assert_eq!(duree_de_cache(set), Some(DUREE_YGOPRODECK));
/// // Le catalogue complet EST la mise à jour ; la version dit s'il en faut une.
/// assert_eq!(duree_de_cache("https://db.ygoprodeck.com/api/v7/cardinfo.php?includeAliased=true"), None);
/// assert_eq!(duree_de_cache("https://db.ygoprodeck.com/api/v7/checkDBVer.php"), None);
/// // L'existence des fichiers se revérifie à chaque mise à jour.
/// assert_eq!(duree_de_cache("https://yugipedia.com/api.php?action=query&prop=imageinfo&titles=File%3AX.png"), None);
/// // Les images ne sont pas des réponses d'API : elles ont leurs fichiers.
/// assert_eq!(duree_de_cache("https://ms.yugipedia.com//a/ab/X.png"), None);
/// ```
#[must_use]
pub fn duree_de_cache(url: &str) -> Option<Duration> {
    let minuscule = url.to_ascii_lowercase();
    let sans_schema = minuscule
        .split_once("://")
        .map_or(minuscule.as_str(), |(_, r)| r);
    let (hote, chemin) = sans_schema.split_once('/').unwrap_or((sans_schema, ""));
    if (hote == "yugipedia.com" || hote == "www.yugipedia.com") && chemin.starts_with("api.php") {
        // La vérification d'existence des fichiers est faite pour voir ce
        // qui a changé, une fois par mise à jour de la base : la garder
        // trente jours lui ferait répondre « absent » à un fichier ajouté
        // la veille.
        if chemin.contains("prop=imageinfo") {
            return None;
        }
        return Some(DUREE_YUGIPEDIA);
    }
    if hote == "db.ygoprodeck.com" {
        if chemin.starts_with("api/v7/cardsets.php") {
            return Some(DUREE_YGOPRODECK);
        }
        if chemin.starts_with("api/v7/cardinfo.php") && chemin.contains("cardset=") {
            return Some(DUREE_YGOPRODECK);
        }
    }
    None
}

/// Nom de fichier d'une URL : FNV-1a 64 bits en hexadécimal.
///
/// Stable d'une version de Rust à l'autre — ce que `DefaultHasher` ne promet
/// pas, et un cache qui changerait de clé à chaque compilation serait vide.
#[must_use]
pub fn cle(url: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for octet in url.as_bytes() {
        h ^= u64::from(*octet);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Le cache d'une installation.
#[derive(Debug)]
pub struct CacheDisque {
    dossier: PathBuf,
    echecs: Mutex<HashMap<String, u64>>,
}

impl CacheDisque {
    /// Ouvre — et crée au besoin — le cache dans ce dossier.
    #[must_use]
    pub fn ouvrir(dossier: &Path) -> Self {
        let _ = std::fs::create_dir_all(dossier.join("reponses"));
        let echecs = std::fs::read(dossier.join("introuvables.json"))
            .ok()
            .and_then(|o| serde_json::from_slice(&o).ok())
            .unwrap_or_default();
        Self {
            dossier: dossier.to_path_buf(),
            echecs: Mutex::new(echecs),
        }
    }

    fn chemin(&self, url: &str) -> PathBuf {
        self.dossier
            .join("reponses")
            .join(format!("{}.json", cle(url)))
    }

    /// La réponse gardée pour cette URL, si elle a moins de `duree`.
    #[must_use]
    pub fn lire(&self, url: &str, duree: Duration) -> Option<Vec<u8>> {
        let chemin = self.chemin(url);
        let age = std::fs::metadata(&chemin)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())?;
        if age > duree {
            return None;
        }
        std::fs::read(chemin).ok()
    }

    /// Garde la réponse de cette URL. Écriture atomique : un fichier à moitié
    /// écrit ne doit jamais passer pour une réponse.
    pub fn ecrire(&self, url: &str, octets: &[u8]) {
        let chemin = self.chemin(url);
        let provisoire = chemin.with_extension("tmp");
        if std::fs::write(&provisoire, octets).is_ok()
            && std::fs::rename(&provisoire, &chemin).is_err()
        {
            let _ = std::fs::remove_file(&provisoire);
        }
    }

    /// Cette adresse a-t-elle été trouvée absente il y a moins de
    /// [`DUREE_ECHEC`] ?
    #[must_use]
    pub fn introuvable_recemment(&self, url: &str) -> bool {
        let Ok(echecs) = self.echecs.lock() else {
            return false;
        };
        echecs
            .get(url)
            .is_some_and(|quand| maintenant().saturating_sub(*quand) < DUREE_ECHEC.as_secs())
    }

    /// Retient qu'une adresse est introuvable, et le garde sur le disque.
    pub fn noter_introuvable(&self, url: &str) {
        let Ok(mut echecs) = self.echecs.lock() else {
            return;
        };
        let limite = maintenant().saturating_sub(DUREE_ECHEC.as_secs());
        echecs.retain(|_, quand| *quand >= limite);
        echecs.insert(url.to_owned(), maintenant());
        self.sauver(&echecs);
    }

    /// Une adresse retrouvée n'est plus introuvable — en mémoire **et** sur
    /// le disque.
    ///
    /// Corrigé le 2026-10-02 : l'oubli ne touchait que la mémoire. Une adresse
    /// retrouvée restait donc « introuvable » au démarrage suivant, jusqu'à
    /// la fin de ses sept jours — et la reprise des replis après une mise à
    /// jour de la base ([`Self::oublier_introuvable`] appelé exprès) n'aurait
    /// servi à rien.
    pub fn oublier_introuvable(&self, url: &str) {
        let Ok(mut echecs) = self.echecs.lock() else {
            return;
        };
        if echecs.remove(url).is_some() {
            self.sauver(&echecs);
        }
    }

    fn sauver(&self, echecs: &HashMap<String, u64>) {
        if let Ok(octets) = serde_json::to_vec(echecs) {
            let chemin = self.dossier.join("introuvables.json");
            let provisoire = chemin.with_extension("tmp");
            if std::fs::write(&provisoire, octets).is_ok() {
                let _ = std::fs::rename(&provisoire, &chemin);
            }
        }
    }
}

fn maintenant() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

static CACHE: OnceLock<CacheDisque> = OnceLock::new();

/// Active le cache pour tout le programme. Le premier appel décide ; les
/// suivants sont sans effet.
///
/// L'interface et `ygo-cli` l'appellent dès qu'ils connaissent l'installation.
/// Les tests ne l'appellent pas : sans cache, chaque requête part — ce qui est
/// le comportement que leurs serveurs locaux attendent.
pub fn activer(dossier: &Path) {
    let _ = CACHE.set(CacheDisque::ouvrir(dossier));
}

/// Le cache du programme, s'il a été activé.
#[must_use]
pub fn actif() -> Option<&'static CacheDisque> {
    CACHE.get()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn la_cle_est_stable_et_distingue_les_urls() {
        // Valeur figée : si elle change, tous les caches existants sont perdus.
        assert_eq!(cle(""), "cbf29ce484222325");
        assert_ne!(cle("https://a"), cle("https://b"));
        assert_eq!(cle("https://a"), cle("https://a"));
    }

    #[test]
    fn une_reponse_gardee_se_relit_jusqu_a_sa_peremption() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = CacheDisque::ouvrir(tmp.path());
        let url = "https://yugipedia.com/api.php?x";
        assert_eq!(cache.lire(url, DUREE_YUGIPEDIA), None);
        cache.ecrire(url, b"{\"a\":1}");
        assert_eq!(
            cache.lire(url, DUREE_YUGIPEDIA).as_deref(),
            Some(&b"{\"a\":1}"[..])
        );
        // Une durée nulle : déjà périmée.
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(cache.lire(url, Duration::ZERO), None);
    }

    #[test]
    fn une_adresse_introuvable_est_retenue_et_survit_a_la_reouverture() {
        let tmp = tempfile::tempdir().unwrap();
        let url = "https://images.ygoprodeck.com/images/cards/1.jpg";
        {
            let cache = CacheDisque::ouvrir(tmp.path());
            assert!(!cache.introuvable_recemment(url));
            cache.noter_introuvable(url);
            assert!(cache.introuvable_recemment(url));
        }
        let rouvert = CacheDisque::ouvrir(tmp.path());
        assert!(rouvert.introuvable_recemment(url), "gardé sur le disque");
        rouvert.oublier_introuvable(url);
        assert!(!rouvert.introuvable_recemment(url));
        // L'oubli aussi survit à la réouverture.
        let encore = CacheDisque::ouvrir(tmp.path());
        assert!(
            !encore.introuvable_recemment(url),
            "oubli gardé sur le disque"
        );
    }
}
