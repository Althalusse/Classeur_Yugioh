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
//! # Quotas — les règles des sources, pour tout le programme
//!
//! | Hôtes | Limite appliquée | Règle de la source |
//! |---|---|---|
//! | `*.ygoprodeck.com` (API **et** images) | **15 req/s, sans rafale** | « 20 requests per 1 second », blocage d'une heure au-delà |
//! | `*.yugipedia.com` (API **et** images) | **1 requête / 1,1 s** | « no more than one per second » |
//! | autres (GitHub…) | aucune | usage ponctuel |
//!
//! # Les limiteurs sont ceux du programme, pas d'un client — 2026-10-01
//!
//! Ils étaient portés par chaque [`ClientHttp`]. Or plusieurs fils en
//! construisent un — celui des téléchargements, celui du contrôle de version,
//! celui des Set lists de l'écran des artworks : deux clients interrogeant
//! Yugipedia en même temps faisaient près de deux requêtes par seconde. Ils
//! sont désormais **statiques**, partagés par tous les clients du processus.
//!
//! Et **sans rafale** : un quota `governor` de 15/s autorise par défaut quinze
//! requêtes d'un coup, puis quinze par seconde — jusqu'à trente dans la même
//! seconde. La rafale est ramenée à une requête : elles sont espacées
//! régulièrement, et aucune fenêtre d'une seconde n'en voit plus que permis.
//!
//! L'application sera partagée : ces limites doivent tenir pour cent
//! utilisateurs comme pour un. Elles ne sont pas des réglages de performance.
//! Voir aussi [`crate::cache`], qui évite de redemander ce qu'on a déjà.

use std::num::NonZeroU32;
use std::sync::OnceLock;
use std::time::Duration;

use governor::clock::DefaultClock;
use governor::state::{InMemoryState, NotKeyed};
use governor::{Quota, RateLimiter};

use crate::error::{Result, SourceError};

/// Où les sources peuvent joindre l'auteur : le dépôt public du projet, dont
/// les tickets sont ouverts.
pub const CONTACT: &str = "https://github.com/Althalusse/Classeur_Yugioh";

/// User-Agent envoyé à toutes les sources.
///
/// Yugipedia : « All automated requests to the API should set a descriptive
/// User-Agent header, including the name of the service and contact
/// information. Requesters who do not do so may be blocked at any time
/// without warning. »
///
/// # Le contact — 2026-10-01
///
/// L'agent pointait vers `github.com/Althalusse/ygo-binder`, qui rend une
/// **404** : un administrateur qui l'aurait suivi tombait dans le vide —
/// exactement le cas où le blocage tombe sans prévenir. Il pointe désormais
/// vers le dépôt réel, public, tickets ouverts : c'est par là qu'on
/// signale un problème à l'auteur, GitHub n'ayant pas de messagerie privée.
pub const USER_AGENT: &str = concat!(
    "YugiohCollectionManager/2.0 (https://github.com/Althalusse/Classeur_Yugioh; ",
    "contact: GitHub issues)"
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

/// Délai global du catalogue YGOPRODeck (`cardinfo.php?includeAliased=true`).
///
/// # Pourquoi un délai à lui — dette 11, 2026-09-30
///
/// Ce n'est pas un appel d'API ordinaire : il rend le catalogue **entier**, et
/// le serveur met longtemps à le fabriquer quand son cache est froid. Mesuré le
/// 2026-09-20 : deux tentatives coupées à 45 s **avant même les en-têtes**,
/// puis une troisième servie en 2 s — 92 s sur les 97 de la mise à jour. Le
/// 5 septembre, cache chaud, tout le catalogue arrivait en 3 s.
///
/// Couper à 45 s ne raccourcit rien : le serveur continue sa fabrication, et
/// l'on revient la chercher. Pire, trois coupures d'affilée rendent le
/// catalogue « indisponible », et la base est alors reconstruite **sans** ses
/// artworks alternatifs ni ses statistiques. 120 s couvrent la fabrication
/// observée (entre 45 et 92 s) avec de la marge ; au-delà, c'est une panne,
/// et les tentatives reprennent leur rôle.
///
/// La connexion, elle, garde ses 8 s : un serveur injoignable échoue toujours
/// vite.
pub const DELAI_CATALOGUE: Duration = Duration::from_secs(120);

/// Nombre de tentatives sur erreur transitoire (`total=3` en Python).
pub const TENTATIVES: u32 = 3;

/// Délai de base du retrait exponentiel (`backoff_factor=0.5`).
pub const BACKOFF_BASE: Duration = Duration::from_millis(500);

/// Codes de statut qui déclenchent une nouvelle tentative.
///
/// Repris de `status_forcelist=(429, 500, 502, 503, 504)`.
pub const STATUTS_A_REESSAYER: [u16; 5] = [429, 500, 502, 503, 504];

type Limiteur = RateLimiter<NotKeyed, InMemoryState, DefaultClock>;

/// Quotas par famille d'hôtes.
struct Quotas {
    ygoprodeck: Limiteur,
    yugipedia: Limiteur,
}

/// Les quotas du programme — un seul jeu, quel que soit le nombre de clients.
static QUOTAS: OnceLock<Quotas> = OnceLock::new();

fn quotas() -> &'static Quotas {
    QUOTAS.get_or_init(Quotas::new)
}

impl Quotas {
    fn new() -> Self {
        let un = NonZeroU32::MIN;
        Self {
            // 15 par seconde, une à la fois : une requête toutes les 67 ms.
            ygoprodeck: RateLimiter::direct(
                Quota::per_second(NonZeroU32::new(15).unwrap_or(un)).allow_burst(un),
            ),
            // 1,1 s minimum entre deux requêtes, sans rafale possible.
            yugipedia: RateLimiter::direct(
                Quota::with_period(Duration::from_millis(1100))
                    .unwrap_or(Quota::per_second(un))
                    .allow_burst(un),
            ),
        }
    }

    /// Limiteur applicable à une URL, s'il y en a un.
    fn pour(&self, url: &str) -> Option<&Limiteur> {
        let hote = hote_de(url)?;
        if hote.ends_with("yugipedia.com") {
            Some(&self.yugipedia)
        } else if hote.ends_with("ygoprodeck.com") {
            Some(&self.ygoprodeck)
        } else {
            None
        }
    }
}

/// Une seule requête Yugipedia **en vol** à la fois.
///
/// # Pourquoi — 2026-10-02
///
/// `Yugipedia:API` renvoie à l'« API etiquette » de MediaWiki : *« If you make
/// your requests in series rather than in parallel (i.e. wait for the one
/// request to finish before sending a new request, such that you are never
/// making more than one request at a time), then you should definitely be
/// fine. »* Le quota espaçait les **départs** (1,1 s) ; le téléchargeur
/// d'images, lui, garde six téléchargements ouverts. Mesuré sur l'installation
/// réelle : jusqu'à trois images terminées dans la même seconde — des
/// requêtes qui se chevauchent. Le jeton se prend avant l'envoi et se rend
/// **après la lecture du corps**, par l'appelant qui lit ce corps.
static SERIE_YUGIPEDIA: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

/// L'URL vise-t-elle Yugipedia (wiki, API ou serveur d'images) ?
#[must_use]
pub fn est_yugipedia(url: &str) -> bool {
    hote_de(url).is_some_and(|h| h.ends_with("yugipedia.com"))
}

/// Attend son tour pour une requête Yugipedia, et le garde tant que le
/// jeton vit. `None` pour tout autre hôte : rien à attendre.
///
/// À prendre **avant** l'envoi, à rendre **après** la lecture du corps.
pub async fn en_serie(url: &str) -> Option<tokio::sync::SemaphorePermit<'static>> {
    if est_yugipedia(url) {
        SERIE_YUGIPEDIA.acquire().await.ok()
    } else {
        None
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

        Ok(Self { client })
    }

    /// Attend que le quota de l'hôte autorise une requête.
    ///
    /// **Privée au crate**, et appelée en un seul endroit : juste avant chaque
    /// envoi, dans [`Self::get_avec_delai`]. Toute requête passe par là — y
    /// compris chaque nouvelle tentative —, et le limiteur est global : le
    /// pool de six téléchargements d'`ygo-images` ne peut pas le contourner.
    ///
    /// # Pourquoi plus publique — 2026-10-01
    ///
    /// Elle l'était pour qu'`ygo-images` « l'emploie dans ses téléchargements
    /// en parallèle ». Le téléchargeur d'images et celui des couvertures
    /// l'appelaient donc **avant** `get_ok`, qui l'appelle lui-même : deux
    /// créneaux par image. Mesuré sur l'installation réelle à l'ouverture de
    /// `CH01` et `LOCR-JP` : 91 images en 201 s, soit **2,2 s par image** au
    /// lieu de 1,1. Fermer l'accès rend la double attente impossible à
    /// réécrire : elle ne compilerait plus.
    pub(crate) async fn attendre_quota(&self, url: &str) {
        if let Some(limiteur) = quotas().pour(url) {
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
        // Une adresse trouvée absente il y a peu n'est pas redemandée : c'est
        // la règle « une image, une fois » d'YGOPRODeck, appliquée aussi à
        // celles qui n'existent pas.
        if crate::cache::actif().is_some_and(|c| c.introuvable_recemment(url)) {
            tracing::debug!(url, "introuvable récemment — pas redemandé");
            return Err(SourceError::Statut {
                url: url.to_owned(),
                statut: 404,
            });
        }
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
                    if let Some(cache) = crate::cache::actif() {
                        if crate::cache::STATUTS_INTROUVABLES.contains(&statut) {
                            cache.noter_introuvable(url);
                        } else if reponse.status().is_success() {
                            cache.oublier_introuvable(url);
                        }
                    }
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
        self.get_ok_avec_delai(url, DELAI_REQUETE).await
    }

    /// Comme [`Self::get_ok`], avec un délai global explicite.
    pub async fn get_ok_avec_delai(&self, url: &str, delai: Duration) -> Result<reqwest::Response> {
        let reponse = self.get_avec_delai(url, delai).await?;
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
        self.get_json_avec_delai(url, DELAI_REQUETE).await
    }

    /// Comme [`Self::get_json`], avec un délai global explicite.
    ///
    /// Le délai couvre la requête **et** la lecture du corps : c'est la
    /// sémantique du `timeout` de `reqwest` posé sur la requête.
    pub async fn get_json_avec_delai<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        delai: Duration,
    ) -> Result<T> {
        let garde = crate::cache::actif().zip(crate::cache::duree_de_cache(url));
        if let Some((cache, duree)) = garde {
            if let Some(octets) = cache.lire(url, duree) {
                if let Ok(valeur) = serde_json::from_slice(&octets) {
                    tracing::debug!(url, "réponse reprise du cache");
                    return Ok(valeur);
                }
            }
        }
        // Une seule requête Yugipedia en vol : le jeton couvre l'envoi et la
        // lecture du corps.
        let jeton = en_serie(url).await;
        let reponse = self.get_ok_avec_delai(url, delai).await?;
        let octets = reponse
            .bytes()
            .await
            .map_err(|e| SourceError::reseau(url, e))?;
        drop(jeton);
        let valeur =
            serde_json::from_slice(&octets).map_err(|e| SourceError::deserialisation(url, e))?;
        // Gardée seulement une fois lue : une page d'erreur ou un JSON tronqué
        // ne doit pas servir trente jours.
        if let Some((cache, _)) = garde {
            cache.ecrire(url, &octets);
        }
        Ok(valeur)
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

    /// Pas de rafale : seize requêtes YGOPRODeck d'affilée prennent au moins
    /// une seconde — aucune fenêtre d'une seconde n'en voit plus de seize,
    /// sous la limite de vingt de la source.
    #[tokio::test]
    async fn le_quota_ygoprodeck_espace_les_requetes_sans_rafale() {
        let client = ClientHttp::new().unwrap();
        let url = "https://images.ygoprodeck.com/images/cards/1.jpg";

        let debut = std::time::Instant::now();
        for _ in 0..16 {
            client.attendre_quota(url).await;
        }
        assert!(
            debut.elapsed() >= Duration::from_millis(950),
            "16 requêtes en moins d'une seconde : {:?}",
            debut.elapsed()
        );
    }

    /// Deux clients, un seul quota : c'est le défaut qu'avait l'écran des
    /// artworks, avec son propre client à côté de celui des téléchargements.
    #[tokio::test]
    async fn deux_clients_partagent_le_meme_quota_yugipedia() {
        let a = ClientHttp::new().unwrap();
        let b = ClientHttp::new().unwrap();
        let url = "https://ms.yugipedia.com//a/ab/X.png";

        let debut = std::time::Instant::now();
        a.attendre_quota(url).await;
        b.attendre_quota(url).await;
        assert!(
            debut.elapsed() >= Duration::from_millis(1000),
            "le second client a dû attendre le premier : {:?}",
            debut.elapsed()
        );
    }

    /// Un hôte sans règle n'attend rien.
    ///
    /// La borne était de 100 ms, et le test est tombé une fois sous Windows
    /// (2026-10-01) : la machine chargée par la suite entière, le fil a été
    /// suspendu un instant. Une borne de temps serrée mesure la machine, pas le
    /// code. Le test dit maintenant ce qu'il vérifie — aucun limiteur pour cet
    /// hôte — et garde une borne qu'un limiteur ne pourrait pas tenir : cent
    /// requêtes au rythme d'YGOPRODeck prendraient plus de six secondes.
    /// Le second appelant attend que le premier ait rendu son jeton : jamais
    /// deux requêtes Yugipedia en vol.
    #[tokio::test]
    async fn yugipedia_n_a_jamais_deux_requetes_en_vol() {
        let url = "https://ms.yugipedia.com//a/ab/X.png";
        assert!(est_yugipedia(url));
        assert!(est_yugipedia("https://yugipedia.com/api.php?x"));
        assert!(!est_yugipedia("https://images.ygoprodeck.com/x.jpg"));
        assert!(en_serie("https://images.ygoprodeck.com/x.jpg")
            .await
            .is_none());

        let premier = en_serie(url).await.unwrap();
        let second = tokio::spawn(async move {
            let _j = en_serie("https://yugipedia.com/api.php").await;
            std::time::Instant::now()
        });
        tokio::time::sleep(Duration::from_millis(150)).await;
        let rendu = std::time::Instant::now();
        drop(premier);
        let obtenu = second.await.unwrap();
        assert!(
            obtenu >= rendu,
            "le second a attendu que le premier rende son jeton"
        );
    }

    #[tokio::test]
    async fn un_hote_sans_quota_ne_bloque_pas() {
        let url = "https://github.com/x";
        assert!(quotas().pour(url).is_none(), "aucun limiteur pour cet hôte");
        let client = ClientHttp::new().unwrap();
        let debut = std::time::Instant::now();
        for _ in 0..100 {
            client.attendre_quota(url).await;
        }
        assert!(
            debut.elapsed() < Duration::from_secs(2),
            "cent passages sans limiteur : {:?}",
            debut.elapsed()
        );
    }

    #[test]
    fn le_user_agent_est_descriptif() {
        // Exigence Yugipedia : identifier l'application et un moyen de contact.
        assert!(USER_AGENT.contains("YugiohCollectionManager"));
        assert!(USER_AGENT.contains("http"));
        // Le contact est le dépôt réel : celui d'avant rendait une 404.
        assert!(USER_AGENT.contains(CONTACT));
    }
}
