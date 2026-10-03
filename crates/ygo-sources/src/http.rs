// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Client HTTP unique, porteur des quotas.
//!
//! Portage de `module/utilitaire/http_client.py`.
//!
//! # Règle R7 — un seul point de construction
//!
//! Il n'existe qu'un endroit dans tout le programme où un client HTTP est
//! construit : [`ClientHttp::new`]. Tout accès réseau passe par là, et hérite
//! donc des quotas, du User-Agent et des délais. En Python, la même intention
//! passait par la discipline « tous les modules importent `http_get` d'ici » ;
//! ici, c'est le seul type qui expose une méthode de requête.
//!
//! # Ce que `rustls` rend inutile
//!
//! Tout l'appareillage du module Python — contexte SSL unique construit sous
//! verrou, `warmup()` sur le fil principal, adaptateur forçant la réutilisation
//! du contexte, sessions par fil — existait pour une seule raison : la création
//! concurrente de contextes OpenSSL corrompait l'état natif. `rustls` n'a pas
//! d'état global C ; `reqwest::Client` est déjà partageable entre fils et
//! réutilise son pool de connexions. Rien de tout cela n'a besoin d'être porté.
//!
//! # Quotas du §3.7 — non négociables
//!
//! | Hôte | Limite | Nature |
//! |---|---|---|
//! | `db.ygoprodeck.com` | ~20 req/s | limite annoncée de l'API |
//! | `images.ygoprodeck.com` | 15 req/s | limiteur global du projet |
//! | `yugipedia.com` | **1 requête / 1,1 s** | étiquette imposée par le wiki |
//! | autres (GitHub…) | aucune | usage ponctuel |
//!
//! Le débit Yugipedia et le User-Agent descriptif ne sont pas des réglages de
//! performance : c'est la politesse exigée par le wiki. Le cahier des charges
//! les qualifie de non négociables dans le portage.

use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;

use governor::clock::DefaultClock;
use governor::state::{InMemoryState, NotKeyed};
use governor::{Quota, RateLimiter};

use crate::error::{Result, SourceError};

/// User-Agent envoyé à toutes les sources.
///
/// Yugipedia exige un agent descriptif ; on garde la forme du Python en
/// l'actualisant à la version 2.
pub const USER_AGENT: &str = concat!(
    "YugiohCollectionManager/2.0 (https://github.com/Althalusse/ygo-binder; ",
    "gestionnaire de collection personnel)"
);

/// Délai d'établissement de connexion.
///
/// Court volontairement : un handshake TLS qui ne répond pas doit échouer vite
/// plutôt que geler, comme le `connect=8` du Python.
pub const DELAI_CONNEXION: Duration = Duration::from_secs(8);

/// Délai global d'une requête ordinaire.
pub const DELAI_REQUETE: Duration = Duration::from_secs(45);

/// Délai global d'un téléchargement volumineux (archive YGOJSON).
pub const DELAI_TELECHARGEMENT: Duration = Duration::from_secs(600);

/// Nombre de tentatives sur erreur transitoire (`total=3` en Python).
pub const TENTATIVES: u32 = 3;

/// Délai de base du retrait exponentiel (`backoff_factor=0.5`).
pub const BACKOFF_BASE: Duration = Duration::from_millis(500);

/// Codes de statut qui déclenchent une nouvelle tentative.
///
/// Repris de `status_forcelist=(429, 500, 502, 503, 504)`.
pub const STATUTS_A_REESSAYER: [u16; 5] = [429, 500, 502, 503, 504];

type Limiteur = RateLimiter<NotKeyed, InMemoryState, DefaultClock>;

/// Quotas par hôte.
struct Quotas {
    ygoprodeck_api: Limiteur,
    ygoprodeck_images: Limiteur,
    yugipedia: Limiteur,
}

impl Quotas {
    fn new() -> Self {
        // `governor` raisonne en « cellules par période ». Un quota de 20/s
        // autorise 20 requêtes par seconde en régime établi.
        let par_seconde = |n: u32| Quota::per_second(NonZeroU32::new(n).unwrap_or(NonZeroU32::MIN));
        Self {
            ygoprodeck_api: RateLimiter::direct(par_seconde(20)),
            ygoprodeck_images: RateLimiter::direct(par_seconde(15)),
            // 1,1 s minimum entre deux requêtes : exprimé comme une cellule
            // qui se régénère toutes les 1 100 ms, sans rafale possible.
            yugipedia: RateLimiter::direct(
                Quota::with_period(Duration::from_millis(1100))
                    .unwrap_or(Quota::per_second(NonZeroU32::MIN)),
            ),
        }
    }

    /// Limiteur applicable à une URL, s'il y en a un.
    fn pour(&self, url: &str) -> Option<&Limiteur> {
        let hote = hote_de(url)?;
        if hote.ends_with("yugipedia.com") {
            Some(&self.yugipedia)
        } else if hote.starts_with("images.ygoprodeck.com") {
            Some(&self.ygoprodeck_images)
        } else if hote.ends_with("ygoprodeck.com") {
            Some(&self.ygoprodeck_api)
        } else {
            None
        }
    }
}

/// Extrait l'hôte d'une URL, en minuscules.
fn hote_de(url: &str) -> Option<String> {
    let sans_schema = url.split_once("://").map_or(url, |(_, r)| r);
    let hote = sans_schema
        .split(['/', '?', '#'])
        .next()?
        .split('@')
        .next_back()?
        .split(':')
        .next()?;
    (!hote.is_empty()).then(|| hote.to_ascii_lowercase())
}

/// Client HTTP de l'application.
///
/// Clonable à coût nul : tous les clones partagent le pool de connexions et
/// les limiteurs de débit, ce qui est indispensable — un quota par clone ne
/// serait pas un quota.
#[derive(Clone)]
pub struct ClientHttp {
    client: reqwest::Client,
    quotas: Arc<Quotas>,
}

impl std::fmt::Debug for ClientHttp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientHttp").finish_non_exhaustive()
    }
}

impl ClientHttp {
    /// Construit le client — **le seul du programme** (règle R7).
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(DELAI_CONNEXION)
            .timeout(DELAI_REQUETE)
            .pool_max_idle_per_host(16)
            .build()
            .map_err(|e| SourceError::reseau("construction du client", e))?;

        Ok(Self {
            client,
            quotas: Arc::new(Quotas::new()),
        })
    }

    /// Attend que le quota de l'hôte autorise une requête.
    ///
    /// Public parce que `ygo-images` en aura besoin pour ses téléchargements en
    /// parallèle : le limiteur est global, le pool de six ne doit pas pouvoir
    /// le contourner.
    pub async fn attendre_quota(&self, url: &str) {
        if let Some(limiteur) = self.quotas.pour(url) {
            limiteur.until_ready().await;
        }
    }

    /// GET avec quota, tentatives et retrait exponentiel.
    ///
    /// Remplaçant direct de `http_get`. Une erreur de connexion ou un statut
    /// de [`STATUTS_A_REESSAYER`] déclenche une nouvelle tentative, jusqu'à
    /// [`TENTATIVES`] fois. Les autres statuts sont renvoyés tels quels :
    /// l'appelant décide, exactement comme le `raise_on_status=False` du
    /// Python.
    pub async fn get(&self, url: &str) -> Result<reqwest::Response> {
        self.get_avec_delai(url, DELAI_REQUETE).await
    }

    /// Comme [`Self::get`], avec un délai global explicite.
    ///
    /// L'archive YGOJSON pèse plus de cent mégaoctets : elle a besoin d'un
    /// délai bien plus large que les appels d'API.
    pub async fn get_avec_delai(&self, url: &str, delai: Duration) -> Result<reqwest::Response> {
        let mut derniere: Option<SourceError> = None;

        for tentative in 0..TENTATIVES {
            if tentative > 0 {
                // Retrait exponentiel : 0,5 s, 1 s, 2 s…
                let attente = BACKOFF_BASE * 2_u32.pow(tentative - 1);
                tracing::warn!(
                    url,
                    tentative,
                    attente_ms = attente.as_millis(),
                    "nouvelle tentative"
                );
                tokio::time::sleep(attente).await;
            }

            self.attendre_quota(url).await;

            match self.client.get(url).timeout(delai).send().await {
                Ok(reponse) => {
                    let statut = reponse.status().as_u16();
                    if STATUTS_A_REESSAYER.contains(&statut) {
                        derniere = Some(SourceError::Statut {
                            url: url.to_owned(),
                            statut,
                        });
                        continue;
                    }
                    return Ok(reponse);
                }
                Err(e) => {
                    // Un délai dépassé ou une coupure sont transitoires : on
                    // retente. Une URL malformée ne l'est pas, mais la
                    // distinction ne vaut pas le code — trois tentatives sur
                    // une URL invalide coûtent 1,5 s.
                    derniere = Some(SourceError::reseau(url, e));
                }
            }
        }

        Err(derniere.unwrap_or_else(|| SourceError::Statut {
            url: url.to_owned(),
            statut: 0,
        }))
    }

    /// GET renvoyant une erreur sur tout statut hors 2xx.
    pub async fn get_ok(&self, url: &str) -> Result<reqwest::Response> {
        let reponse = self.get(url).await?;
        let statut = reponse.status();
        if !statut.is_success() {
            return Err(SourceError::Statut {
                url: url.to_owned(),
                statut: statut.as_u16(),
            });
        }
        Ok(reponse)
    }

    /// GET désérialisé en JSON typé.
    pub async fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T> {
        let reponse = self.get_ok(url).await?;
        let octets = reponse
            .bytes()
            .await
            .map_err(|e| SourceError::reseau(url, e))?;
        serde_json::from_slice(&octets).map_err(|e| SourceError::deserialisation(url, e))
    }

    /// Accès au client sous-jacent, pour les appelants qui streament eux-mêmes.
    pub fn brut(&self) -> &reqwest::Client {
        &self.client
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    #[test]
    fn extraction_de_l_hote() {
        assert_eq!(
            hote_de("https://yugipedia.com/api.php?a=1").as_deref(),
            Some("yugipedia.com")
        );
        assert_eq!(
            hote_de("https://images.ygoprodeck.com/images/cards/1.jpg").as_deref(),
            Some("images.ygoprodeck.com")
        );
        assert_eq!(
            hote_de("https://db.ygoprodeck.com/api/v7/cardinfo.php").as_deref(),
            Some("db.ygoprodeck.com")
        );
        assert_eq!(hote_de("https://HOST:8443/x").as_deref(), Some("host"));
        assert_eq!(hote_de("pas-une-url").as_deref(), Some("pas-une-url"));
        assert_eq!(hote_de(""), None);
    }

    #[test]
    fn chaque_hote_recoit_son_limiteur() {
        let q = Quotas::new();
        // On ne peut pas comparer des limiteurs ; on vérifie leur présence et
        // qu'un hôte inconnu n'en a pas.
        assert!(q.pour("https://yugipedia.com/api.php").is_some());
        assert!(q.pour("https://images.ygoprodeck.com/x.jpg").is_some());
        assert!(q.pour("https://db.ygoprodeck.com/api/v7/x").is_some());
        assert!(q
            .pour("https://github.com/iconmaster5326/YGOJSON")
            .is_none());
    }

    /// Le quota Yugipedia est le seul dont le respect est une question
    /// d'étiquette et non de performance : on vérifie qu'il impose bien un
    /// espacement, et non une rafale.
    #[tokio::test]
    async fn le_quota_yugipedia_espace_les_requetes() {
        let client = ClientHttp::new().unwrap();
        let url = "https://yugipedia.com/api.php";

        let debut = std::time::Instant::now();
        client.attendre_quota(url).await; // première : immédiate
        client.attendre_quota(url).await; // deuxième : doit attendre ~1,1 s
        let ecoule = debut.elapsed();

        assert!(
            ecoule >= Duration::from_millis(1000),
            "deux requêtes Yugipedia doivent être espacées d'au moins 1 s, mesuré : {ecoule:?}"
        );
    }

    #[tokio::test]
    async fn le_quota_ygoprodeck_autorise_une_rafale_de_vingt() {
        let client = ClientHttp::new().unwrap();
        let url = "https://db.ygoprodeck.com/api/v7/cardinfo.php";

        let debut = std::time::Instant::now();
        for _ in 0..20 {
            client.attendre_quota(url).await;
        }
        assert!(
            debut.elapsed() < Duration::from_millis(500),
            "20 requêtes doivent passer dans la première seconde"
        );
    }

    #[tokio::test]
    async fn un_hote_sans_quota_ne_bloque_pas() {
        let client = ClientHttp::new().unwrap();
        let debut = std::time::Instant::now();
        for _ in 0..100 {
            client.attendre_quota("https://github.com/x").await;
        }
        assert!(debut.elapsed() < Duration::from_millis(100));
    }

    #[test]
    fn le_user_agent_est_descriptif() {
        // Exigence Yugipedia : identifier l'application et un moyen de contact.
        assert!(USER_AGENT.contains("YugiohCollectionManager"));
        assert!(USER_AGENT.contains("http"));
    }
}
