//! Le client et sa mémoire des adresses introuvables, de bout en bout.
//!
//! Un fichier de test à part : le cache est global au processus, et
//! l'activer dans les tests unitaires changerait le comportement des autres.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::io::{Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use ygo_sources::ClientHttp;

/// Un serveur local qui répond 404 à tout, et compte les connexions.
fn serveur_404() -> (String, Arc<AtomicUsize>) {
    let ecoute = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let adresse = ecoute.local_addr().unwrap();
    let connexions = Arc::new(AtomicUsize::new(0));
    let compte = connexions.clone();
    std::thread::spawn(move || {
        for flux in ecoute.incoming() {
            let Ok(mut flux) = flux else { break };
            compte.fetch_add(1, Ordering::SeqCst);
            let mut tampon = [0_u8; 2048];
            let _ = flux.read(&mut tampon);
            let _ = flux.write_all(
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        }
    });
    (format!("http://{adresse}/images/cards/123.jpg"), connexions)
}

/// Une image introuvable n'est demandée qu'une fois, et la mémoire survit à
/// un redémarrage — c'est ce qui manquait : chaque ouverture de classeur
/// redemandait les mêmes images absentes.
#[tokio::test]
async fn une_adresse_introuvable_n_est_demandee_qu_une_fois() {
    let tmp = tempfile::tempdir().unwrap();
    ygo_sources::cache::activer(tmp.path());
    let client = ClientHttp::new().unwrap();
    let (url, connexions) = serveur_404();

    let premiere = client.get_ok(&url).await;
    assert!(premiere.is_err());
    assert_eq!(connexions.load(Ordering::SeqCst), 1);

    let seconde = client.get_ok(&url).await;
    assert!(seconde.is_err(), "toujours introuvable");
    assert_eq!(
        connexions.load(Ordering::SeqCst),
        1,
        "pas redemandée avant sept jours"
    );

    // Retenue sur le disque, pour la prochaine session.
    let rouvert = ygo_sources::cache::CacheDisque::ouvrir(tmp.path());
    assert!(rouvert.introuvable_recemment(&url));
}
