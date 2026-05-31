"""
file_attente_classeur.py — File d'attente pour la création et le téléchargement de classeurs.

Singleton thread-safe : instancié une seule fois pour toute la durée de l'application.
Le worker tourne en arrière-plan et traite les tâches une par une.
"""

# Yu-Gi-Oh! Collection Manager
# Copyright (C) 2026  Althalusse
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License
# along with this program.  If not, see <https://www.gnu.org/licenses/>.

import os
import threading
import queue as queue_module
from enum import Enum
from urllib.parse import urlparse


class StatutTache(Enum):
    EN_ATTENTE = "⏳ En attente"
    EN_COURS   = "🔄 En cours"
    TERMINE    = "✅ Terminé"
    ERREUR     = "❌ Erreur"
    ANNULE     = "🚫 Annulé"


class PhaseTache(Enum):
    """Phase fonctionnelle du traitement, à des fins d'affichage UI.

    Distingue dans le centre d'activité visible (cf. dialog_centre_activite)
    si l'utilisateur regarde une création de classeur (DB vide → DB peuplée)
    ou un téléchargement d'images. Les deux peuvent se chaîner pour une
    même tâche : phase passe de CREATION → TELECHARGEMENT → TERMINEE.

    NB : ce champ est purement informatif côté UI. Le worker continue de
    fonctionner exactement comme avant — la phase est mise à jour avant
    chaque grande étape pour que l'UI puisse afficher un libellé pertinent.
    """
    INITIAL       = "initial"
    CREATION      = "création"
    TELECHARGEMENT = "téléchargement"
    TERMINEE      = "terminée"


class TacheClasseur:
    """Représente un travail de création + téléchargement d'un classeur."""

    def __init__(self, code: str, nom: str = ""):
        self.code               = code
        self.nom                = nom         # Nom complet du set (ex. "Legend of Blue Eyes White Dragon")
        self.statut             = StatutTache.EN_ATTENTE
        self.phase              = PhaseTache.INITIAL
        self.progression        = 0          # 0–100
        self.message            = "En attente..."
        self.annule             = False
        # True si la tâche a été créée par verifier_et_telecharger_images_manquantes
        # (reprise après fermeture ou correction d'anomalie) — distingue des vraies
        # créations de classeurs pour éviter une boucle de refresh infinie.
        self.from_missing_check = False
        # Event signalé par le worker quand la tâche est terminée (succès,
        # erreur ou annulation). Permet à un caller (ex : import CSV) de
        # bloquer le temps que la création soit finie avant de continuer
        # avec la phase d'import des lignes — sans avoir à poller la liste
        # des tâches manuellement.
        self.fini = threading.Event()

    # ── représentation lisible ──────────────────────────────────────────────
    def __repr__(self):
        return f"<TacheClasseur code={self.code!r} statut={self.statut.name} phase={self.phase.name}>"


class FileAttenteClasseur:
    """
    Singleton gérant la file d'attente des créations de classeurs.

    Usage :
        file = FileAttenteClasseur()
        file.definir_callback_refresh(ma_fonction)
        tache = file.ajouter("LOB")
    """

    _instance = None
    _lock_singleton = threading.Lock()

    def __new__(cls):
        with cls._lock_singleton:
            if cls._instance is None:
                inst = super().__new__(cls)
                inst._initialise = False
                cls._instance = inst
        return cls._instance

    def __init__(self):
        if self._initialise:
            return
        self._initialise        = True
        self.taches: list[TacheClasseur] = []
        self._file: queue_module.Queue   = queue_module.Queue()
        self._thread: threading.Thread | None = None
        self._callback_refresh  = None   # appelé depuis n'importe quel thread
        self._lock_taches       = threading.Lock()

    # ── API publique ─────────────────────────────────────────────────────────

    def definir_callback_refresh(self, callback):
        """
        Définit la fonction appelée à chaque changement d'état.
        Le callback doit être thread-safe (utiliser .after() côté tkinter).
        """
        self._callback_refresh = callback

    def ajouter(self, code: str, nom: str = "") -> TacheClasseur:
        """
        Ajoute un classeur à la file.
        Si ce code est déjà en attente ou en cours, retourne la tâche existante.
        """
        code = code.strip().upper()
        with self._lock_taches:
            for t in self.taches:
                if t.code == code and t.statut in (
                    StatutTache.EN_ATTENTE, StatutTache.EN_COURS
                ):
                    return t  # Déjà en file, pas de doublon
            tache = TacheClasseur(code, nom)
            self.taches.append(tache)

        self._file.put(tache)
        self._notifier()
        self._demarrer_worker()
        return tache

    def vider_termines(self):
        """Retire de la liste les tâches finies (terminées, erreurs, annulées)."""
        with self._lock_taches:
            self.taches = [
                t for t in self.taches
                if t.statut in (StatutTache.EN_ATTENTE, StatutTache.EN_COURS)
            ]
        self._notifier()

    def nb_total_actives(self) -> int:
        """Nombre de tâches en attente + en cours (utile pour le badge UI)."""
        with self._lock_taches:
            return sum(
                1 for t in self.taches
                if t.statut in (StatutTache.EN_ATTENTE, StatutTache.EN_COURS)
            )

    def snapshot_taches(self) -> list[TacheClasseur]:
        """Retourne une copie de la liste des tâches (lecture thread-safe).

        Important : la copie est superficielle — chaque tâche reste l'objet
        partagé avec le worker, donc ses champs (statut, progression,
        message) restent vivants et reflètent les changements futurs.
        Mais la LISTE retournée ne change pas si le caller la stocke,
        contrairement à self.taches qui est mutée par ajouter/vider_termines.
        """
        with self._lock_taches:
            return list(self.taches)

    def attendre_taches(self, taches: list[TacheClasseur],
                        timeout: float | None = None) -> bool:
        """Bloque jusqu'à ce que toutes les tâches données soient terminées.

        Utilisé typiquement par l'import CSV (phase 1 : créer les classeurs
        absents via la file d'attente, puis attendre qu'ils soient prêts
        avant d'enchaîner avec la phase 2 — l'import des lignes en BDD).

        Args:
            taches  : liste de TacheClasseur retournées par ajouter().
            timeout : durée maximale d'attente totale (en secondes), ou
                      None pour attendre indéfiniment. Le timeout s'applique
                      à l'attente CUMULÉE, pas par tâche.

        Returns:
            True si toutes les tâches sont terminées dans les temps,
            False si le timeout a expiré.

        Pas thread-safe sur la liste `taches` : le caller doit la passer
        en lecture seule (généralement, il vient juste de l'obtenir via
        une succession d'appels à ajouter()).
        """
        import time
        if not taches:
            return True
        if timeout is None:
            for t in taches:
                t.fini.wait()
            return True
        # Timeout cumulé : on calcule le temps restant à chaque tâche.
        deadline = time.monotonic() + timeout
        for t in taches:
            restant = deadline - time.monotonic()
            if restant <= 0:
                return t.fini.is_set()  # déjà fini ou pas, on n'attend plus
            if not t.fini.wait(timeout=restant):
                return False
        return True

    # ── Thread worker ────────────────────────────────────────────────────────

    def _demarrer_worker(self):
        with self._lock_singleton:
            if self._thread is None or not self._thread.is_alive():
                self._thread = threading.Thread(
                    target=self._worker, daemon=True, name="FileAttenteWorker"
                )
                self._thread.start()

    def _worker(self):
        """Traite les tâches une par une jusqu'à ce que la file soit vide."""
        # Imports lourds faits ici pour ne pas ralentir le démarrage de l'app
        from module.creation_classeur.creation_classeur_service import (
            create_classeur, classeur_db_est_peuple,
        )
        from module.img_dl.telechargement_service import TelechargementService
        from module.centralisation_dossier import (
            CLASSEUR_FOLDER, IMG_FOLDER, sqlite_ctx
        )
        import shutil

        service = TelechargementService()

        while True:
            # Attente d'une tâche (timeout = fin du worker si file vide)
            try:
                tache: TacheClasseur = self._file.get(timeout=5)
            except queue_module.Empty:
                break

            # Bloc try/finally global : garantit que tache.fini est signalé
            # sur TOUS les chemins de sortie (succès, erreur, annulation,
            # exception inattendue). C'est crucial pour les callers qui
            # attendent la fin via tache.fini.wait() — un oubli laisserait
            # le caller bloqué pour toujours.
            try:
                if tache.annule:
                    tache.statut = StatutTache.ANNULE
                    tache.phase  = PhaseTache.TERMINEE
                    self._notifier()
                    continue

                # ── Étape 1 : Création du classeur (DB) ─────────────────────
                # Si le classeur existe déjà sur disque (ex : re-téléchargement
                # déclenché après correction d'anomalie), on saute la création
                # et on passe directement au téléchargement des images
                # manquantes.
                tache.statut      = StatutTache.EN_COURS
                tache.progression = 0
                classeur_path = os.path.join(CLASSEUR_FOLDER, tache.code)

                if classeur_db_est_peuple(tache.code):
                    # Classeur déjà peuplé — aller directement au téléchargement
                    tache.phase   = PhaseTache.TELECHARGEMENT
                    tache.message = "Vérification des images manquantes..."
                    self._notifier()
                else:
                    tache.phase   = PhaseTache.CREATION
                    tache.message = "Création du classeur..."
                    self._notifier()

                    try:
                        ok = create_classeur(tache.code)
                    except ValueError as ve:
                        tache.statut  = StatutTache.ERREUR
                        tache.phase   = PhaseTache.TERMINEE
                        tache.message = str(ve)
                        self._notifier()
                        continue
                    except Exception as e:
                        tache.statut  = StatutTache.ERREUR
                        tache.phase   = PhaseTache.TERMINEE
                        tache.message = f"Erreur création : {e}"
                        self._notifier()
                        continue

                    if not ok:
                        tache.statut  = StatutTache.ERREUR
                        tache.phase   = PhaseTache.TERMINEE
                        tache.message = "Échec de la création (DB vide ?)"
                        self._notifier()
                        continue

                # ── Étape 2 : Téléchargement des images ─────────────────────
                tache.phase       = PhaseTache.TELECHARGEMENT
                tache.progression = 10
                tache.message     = "Récupération des URLs..."
                self._notifier()

                try:
                    from module.config_image_source import build_image_url, build_fallback_url
                    from module.centralisation_dossier import IMAGES_SMALL_FOLDER
                    from module.gestion_img.gestion_image_classeur import est_notfound_placeholder

                    db_path = os.path.join(CLASSEUR_FOLDER, tache.code, f"{tache.code}.db")

                    # Pool partagé (spec §5) : img/small/{card_id}.jpg
                    # Fallback per-classeur pour anciens classeurs YGOJSON.
                    os.makedirs(IMAGES_SMALL_FOLDER, exist_ok=True)

                    with sqlite_ctx(db_path) as conn:
                        cursor = conn.cursor()
                        cursor.execute("""
                            SELECT card_image_url, card_image_id
                            FROM cards
                            WHERE card_image_url IS NOT NULL
                               OR card_image_id  IS NOT NULL
                        """)
                        url_rows = cursor.fetchall()

                    # Construction de la liste à (re)télécharger.
                    #
                    # Une entrée y figure si :
                    #   - Le fichier local est absent, OU
                    #   - Le fichier local est un placeholder notfound.jpg posé
                    #     lors d'une tentative précédente (→ retry à l'ouverture).
                    #
                    # Pour chaque carte on calcule EN AVANCE l'URL primaire et
                    # l'URL de fallback (alternative). Le worker essaie ensuite
                    # primaire → fallback → placeholder notfound via
                    # telecharger_avec_fallback().
                    a_telecharger = []  # liste de (url_primary, url_fallback, dest_path)
                    for (stored_url, stored_id) in url_rows:
                        url_primary  = build_image_url(stored_url, stored_id)
                        if not url_primary:
                            continue
                        url_fallback = build_fallback_url(stored_url, stored_id)
                        filename = os.path.basename(urlparse(url_primary).path)
                        if stored_id:
                            dest_path = os.path.join(IMAGES_SMALL_FOLDER, filename)
                        else:
                            per_classeur = os.path.join(IMG_FOLDER, tache.code)
                            os.makedirs(per_classeur, exist_ok=True)
                            dest_path = os.path.join(per_classeur, filename)

                        if not os.path.exists(dest_path):
                            # Image jamais téléchargée
                            a_telecharger.append((url_primary, url_fallback, dest_path))
                        elif est_notfound_placeholder(dest_path):
                            # Image remplacée par un placeholder lors d'une
                            # tentative précédente — on retente à cette ouverture.
                            a_telecharger.append((url_primary, url_fallback, dest_path))

                    total = len(a_telecharger)

                    if total == 0:
                        tache.statut      = StatutTache.TERMINE
                        tache.phase       = PhaseTache.TERMINEE
                        tache.progression = 100
                        tache.message     = "Images déjà présentes ✓"
                        self._notifier()
                        continue

                    tache.message = f"Téléchargement de {total} image(s)..."
                    self._notifier()

                    nb_ok   = 0  # téléchargements réussis (primaire ou fallback)
                    nb_fail = 0  # placeholder notfound posé
                    for i, (url_p, url_f, dest_path) in enumerate(a_telecharger, 1):
                        if tache.annule:
                            break
                        try:
                            ok = service.telecharger_avec_fallback(url_p, url_f, dest_path)
                            if ok:
                                nb_ok += 1
                            else:
                                nb_fail += 1
                        except Exception:
                            # Dernier filet de sécurité : telecharger_avec_fallback
                            # ne doit normalement jamais lever (il pose un placeholder),
                            # mais on reste défensif pour ne pas tuer le worker.
                            nb_fail += 1

                        # Mise à jour progression (10 % réservés à la création DB)
                        tache.progression = 10 + int((i / total) * 90)
                        tache.message     = f"Image {i}/{total}"
                        # Notifier seulement tous les 5 téléchargements (perf UI)
                        if i % 5 == 0 or i == total:
                            self._notifier()

                    if tache.annule:
                        tache.statut  = StatutTache.ANNULE
                        tache.phase   = PhaseTache.TERMINEE
                        tache.message = "Annulé par l'utilisateur"
                    else:
                        tache.statut      = StatutTache.TERMINE
                        tache.phase       = PhaseTache.TERMINEE
                        tache.progression = 100
                        if nb_fail == 0:
                            tache.message = f"{nb_ok} image(s) téléchargée(s) ✓"
                        elif nb_ok == 0:
                            tache.message = f"{nb_fail} image(s) indisponible(s) (réessai à la prochaine ouverture)"
                        else:
                            tache.message = (
                                f"{nb_ok} image(s) téléchargée(s), "
                                f"{nb_fail} indisponible(s)"
                            )

                except Exception as e:
                    tache.statut  = StatutTache.ERREUR
                    tache.phase   = PhaseTache.TERMINEE
                    tache.message = f"Erreur téléchargement : {e}"

                self._notifier()
            finally:
                # Quoi qu'il se passe (continue, exception, fin normale),
                # on signale toujours la fin pour débloquer les waiters et
                # libérer le slot dans la queue.
                try:
                    self._file.task_done()
                except Exception:
                    # task_done() peut lever ValueError si appelé en trop
                    # (ne devrait jamais arriver ici car on get() une seule
                    # fois par itération, mais ceinture+bretelles).
                    pass
                tache.fini.set()

    # ── Notification UI ──────────────────────────────────────────────────────

    def _notifier(self):
        if self._callback_refresh:
            try:
                self._callback_refresh()
            except Exception:
                pass
