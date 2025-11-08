"""
Module utilitaire pour centraliser la logique de rafraîchissement des vues et callbacks dans l'application Yu-Gi-Oh! Classeur.
Inclut des helpers pour le rafraîchissement manuel, périodique, et la création de callbacks multiples.
"""
import tkinter as tk


def creer_callback_rafraichir(*fonctions):
    """
    Retourne un callback qui appelle en séquence toutes les fonctions passées en argument.
    Utile pour lier plusieurs rafraîchissements à un même événement (ex : changement d'onglet).
    """
    def callback():
        for f in fonctions:
            if callable(f):
                f()
    return callback


def rafraichir_si_onglet(event, mapping):
    """
    Callback générique à utiliser sur <<NotebookTabChanged>>.
    mapping : dict {nom_onglet: fonction_rafraichir}
    """
    selected_tab = event.widget.tab(event.widget.index("current"))["text"]
    if selected_tab in mapping:
        mapping[selected_tab]()


def ajouter_bouton_rafraichir(parent, callback, **kwargs):
    """
    Ajoute un bouton de rafraîchissement standardisé à un parent.
    Retourne le bouton créé.
    """
    from tkinter import ttk
    btn = ttk.Button(parent, text="Rafraîchir", command=callback, **kwargs)
    btn.pack(pady=5)
    return btn


def rafraichissement_periodique(widget, callback, interval_ms=2000):
    """
    Lance un rafraîchissement périodique sur un widget (ex: frame, root).
    """
    def periodic():
        callback()
        widget.after(interval_ms, periodic)
    widget.after(interval_ms, periodic)


def setup_rafraichissement_inventaire(frame, refresh_callback):
    """
    Configure le rafraîchissement périodique pour l'inventaire.
    Le bouton de rafraîchissement manuel est maintenant géré dans l'interface utilisateur.
    """
    rafraichissement_periodique(frame, refresh_callback)


def refresh_classeurs(instance): #pour ajout_carte.py
    """
    Fonction appelée depuis ajout_carte.py pour rafraîchir la liste des classeurs dans les combobox.
    """
    current = instance.combo_classeur.get()
    current_filter = instance.custom_filter.get()
    classeurs = instance.get_classeurs()
    instance.combo_classeur['values'] = classeurs
    instance.custom_filter['values'] = ["Tous"] + classeurs
    if current in classeurs:
        instance.combo_classeur.set(current)
    if current_filter in ["Tous"] + classeurs:
        instance.custom_filter.set(current_filter)
    else:
        instance.custom_filter.set("Tous")
    if instance.refresh_callback:
        # Utilise le callback centralisé si besoin
        creer_callback_rafraichir(instance.refresh_callback)()
