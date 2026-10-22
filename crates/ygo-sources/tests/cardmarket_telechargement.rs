//! Les fichiers Cardmarket, de bout en bout contre un serveur local : une
//! vérification par jour, requête conditionnelle, copie précédente gardée
//! quand le serveur sert n'importe quoi.

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use ygo_sources::cardmarket::{self, Fichier, Issue};
use ygo_sources::ClientHttp;

/// Ce que le serveur sert, et ce qu'il a vu.
#[derive(Default)]
struct Scene {
    /// Par chemin : (ETag, corps).
    fichiers: HashMap<String, (String, String)>,
    /// Requêtes reçues, avec leur `If-None-Match`.
    requetes: Vec<(String, Option<String>)>,
}

fn serveur(scene: Arc<Mutex<Scene>>) -> String {
    let ecoute = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let adresse = format!("http://{}", ecoute.local_addr().unwrap());
    std::thread::spawn(move || {
        for flux in ecoute.incoming() {
            let Ok(mut flux) = flux else { continue };
            let mut tampon = Vec::new();
            let mut morceau = [0_u8; 1024];
            while !tampon.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = flux.read(&mut morceau).unwrap_or(0);
                if n == 0 {
                    break;
                }
                tampon.extend_from_slice(&morceau[..n]);
            }
            let requete = String::from_utf8_lossy(&tampon).to_string();
            let chemin = requete.split_whitespace().nth(1).unwrap_or("/").to_owned();
            let condition = requete.lines().find_map(|l| {
                let (nom, valeur) = l.split_once(':')?;
                nom.eq_ignore_ascii_case("if-none-match")
                    .then(|| valeur.trim().to_owned())
            });
            let mut s = scene.lock().unwrap();
            s.requetes.push((chemin.clone(), condition.clone()));
            let reponse = match s.fichiers.get(&chemin) {
                Some((etag, _)) if condition.as_deref() == Some(etag.as_str()) => {
                    "HTTP/1.1 304 Not Modified\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        .to_owned()
                }
                Some((etag, corps)) => format!(
                    "HTTP/1.1 200 OK\r\nETag: {etag}\r\nLast-Modified: Tue, 20 Oct 2026 00:45:00 GMT\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{corps}",
                    corps.len()
                ),
                None => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .to_owned(),
            };
            drop(s);
            let _ = flux.write_all(reponse.as_bytes());
        }
    });
    adresse
}

fn chemin(f: Fichier) -> String {
    f.url().replacen(cardmarket::BASE, "", 1)
}

fn scene_initiale() -> Arc<Mutex<Scene>> {
    let mut s = Scene::default();
    for f in Fichier::TOUS {
        s.fichiers.insert(
            chemin(f),
            (
                format!("\"v1-{}\"", f.nom()),
                format!(
                    "{{\"version\":1,\"createdAt\":\"jour-1\",\"products\":[],\"fichier\":\"{}\"}}",
                    f.nom()
                ),
            ),
        );
    }
    Arc::new(Mutex::new(s))
}

#[tokio::test]
async fn une_verification_par_jour_et_des_requetes_conditionnelles() {
    let scene = scene_initiale();
    let base = serveur(scene.clone());
    let client = ClientHttp::new().unwrap();
    let dir = tempfile::tempdir().unwrap();

    // 1. Premier passage : les trois fichiers descendent.
    let b = cardmarket::telecharger_depuis(&client, dir.path(), false, &base)
        .await
        .unwrap();
    assert!(b.a_change());
    assert!(b
        .fichiers
        .iter()
        .all(|(_, i)| matches!(i, Issue::Recu { .. })));
    for f in Fichier::TOUS {
        assert!(dir.path().join(f.nom()).is_file(), "{}", f.nom());
    }
    assert_eq!(scene.lock().unwrap().requetes.len(), 3);
    assert!(
        scene
            .lock()
            .unwrap()
            .requetes
            .iter()
            .all(|(_, c)| c.is_none()),
        "rien à présenter la première fois"
    );

    // 2. Le même jour : aucune requête.
    let b = cardmarket::telecharger_depuis(&client, dir.path(), false, &base)
        .await
        .unwrap();
    assert!(b.fichiers.iter().all(|(_, i)| *i == Issue::PasEncore));
    assert_eq!(scene.lock().unwrap().requetes.len(), 3, "pas une de plus");

    // 3. Forcé : la requête part, conditionnelle — rien ne redescend.
    let b = cardmarket::telecharger_depuis(&client, dir.path(), true, &base)
        .await
        .unwrap();
    assert!(!b.a_change());
    assert!(b.fichiers.iter().all(|(_, i)| *i == Issue::Inchange));
    {
        let s = scene.lock().unwrap();
        assert_eq!(s.requetes.len(), 6);
        assert!(s.requetes[3..]
            .iter()
            .all(|(_, c)| c.as_deref().is_some_and(|e| e.starts_with("\"v1-"))));
    }

    // 4. Le guide des prix change : lui seul redescend.
    {
        let mut s = scene.lock().unwrap();
        s.fichiers.insert(
            chemin(Fichier::Prix),
            (
                "\"v2\"".to_owned(),
                "{\"version\":1,\"createdAt\":\"jour-2\",\"priceGuides\":[]}".to_owned(),
            ),
        );
    }
    let b = cardmarket::telecharger_depuis(&client, dir.path(), true, &base)
        .await
        .unwrap();
    let prix = b
        .fichiers
        .iter()
        .find(|(f, _)| *f == Fichier::Prix)
        .map(|(_, i)| i.clone())
        .unwrap();
    assert!(matches!(prix, Issue::Recu { .. }));
    assert_eq!(
        cardmarket::date_du_fichier(&dir.path().join(Fichier::Prix.nom())).as_deref(),
        Some("jour-2")
    );
    let etat = cardmarket::Etat::charger(dir.path());
    assert_eq!(
        etat.de(Fichier::Prix).unwrap().etag.as_deref(),
        Some("\"v2\"")
    );
}

/// Une page d'erreur servie en 200 ne remplace pas le bon fichier de la veille.
#[tokio::test]
async fn un_contenu_qui_n_est_pas_du_json_ne_remplace_rien() {
    let scene = scene_initiale();
    let base = serveur(scene.clone());
    let client = ClientHttp::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    cardmarket::telecharger_depuis(&client, dir.path(), false, &base)
        .await
        .unwrap();
    let avant = std::fs::read(dir.path().join(Fichier::Cartes.nom())).unwrap();

    scene.lock().unwrap().fichiers.insert(
        chemin(Fichier::Cartes),
        (
            "\"v2\"".to_owned(),
            "<!DOCTYPE html><p>maintenance</p>".to_owned(),
        ),
    );
    let b = cardmarket::telecharger_depuis(&client, dir.path(), true, &base)
        .await
        .unwrap();
    let cartes = b
        .fichiers
        .iter()
        .find(|(f, _)| *f == Fichier::Cartes)
        .map(|(_, i)| i.clone())
        .unwrap();
    assert!(matches!(cartes, Issue::Echec(_)), "{cartes:?}");
    assert_eq!(
        std::fs::read(dir.path().join(Fichier::Cartes.nom())).unwrap(),
        avant,
        "la copie de la veille est intacte"
    );
    assert_eq!(b.echecs().len(), 1);
}
