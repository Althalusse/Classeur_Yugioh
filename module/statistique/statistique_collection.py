from .statistique_collection_service import get_stats_collection, stats_par_collection
import tkinter as tk
from tkinter import ttk

def afficher_stats():
    stats = get_stats_collection()
    print(f"{'Collection':<12} | {'Possédées':<10} | {'Total':<6} | {'% Complétion':<12}")
    print("-" * 50)
    for stat in stats:
        print(f"{stat['nom']:<12} | {stat['possedees']:<10} | {stat['total']:<6} | {stat['pourcentage']:10.2f} %")

def afficher_stats_interface(cadre):
    """
    Affiche l'interface graphique des statistiques de collection dans le widget 'cadre'.
    """
    # Efface le contenu précédent
    for widget in cadre.winfo_children():
        widget.destroy()
    panneau_principal = ttk.PanedWindow(cadre, orient=tk.HORIZONTAL)
    panneau_principal.pack(fill=tk.BOTH, expand=True)
    panneau_gauche = ttk.Frame(panneau_principal)
    panneau_principal.add(panneau_gauche, weight=2)
    panneau_droite = ttk.Frame(panneau_principal)
    panneau_principal.add(panneau_droite, weight=1)

    def afficher_stats_globales():
        for widget in panneau_gauche.winfo_children():
            widget.destroy()
        stats = get_stats_collection()
        canvas = tk.Canvas(panneau_gauche)
        scrollbar = ttk.Scrollbar(panneau_gauche, orient="vertical", command=canvas.yview)
        cadre_defilable = ttk.Frame(canvas)
        cadre_defilable.bind(
            "<Configure>",
            lambda e: canvas.configure(scrollregion=canvas.bbox("all"))
        )
        canvas.create_window((0, 0), window=cadre_defilable, anchor="nw")
        canvas.configure(yscrollcommand=scrollbar.set)
        en_tete = ttk.Frame(cadre_defilable)
        en_tete.pack(fill=tk.X, pady=5)
        ttk.Label(en_tete, text="Collection", width=15, font=('Helvetica', 10, 'bold')).pack(side=tk.LEFT, padx=2)
        ttk.Label(en_tete, text="Progression", width=40, font=('Helvetica', 10, 'bold')).pack(side=tk.LEFT, padx=2, fill=tk.X, expand=True)
        ttk.Label(en_tete, text="Détails", width=15, font=('Helvetica', 10, 'bold')).pack(side=tk.LEFT, padx=2)
        for stat in stats:
            ligne = ttk.Frame(cadre_defilable, padding=5)
            ligne.pack(fill=tk.X, pady=2)
            ttk.Label(ligne, text=stat['nom'], width=15).pack(side=tk.LEFT, padx=2)
            cadre_progression = ttk.Frame(ligne)
            cadre_progression.pack(side=tk.LEFT, padx=2, fill=tk.X, expand=True)
            style = ttk.Style()
            style.configure("Custom.Horizontal.TProgressbar", 
                          troughcolor='#f0f0f0',
                          background='#4CAF50',
                          thickness=20)
            progression = ttk.Progressbar(cadre_progression, 
                                     orient=tk.HORIZONTAL, 
                                     length=200, 
                                     mode='determinate',
                                     style="Custom.Horizontal.TProgressbar")
            progression['value'] = stat['pourcentage']
            progression.pack(side=tk.LEFT, fill=tk.X, expand=True)
            details = f"{stat['possedees']} / {stat['total']} ({stat['pourcentage']:.1f}%)"
            ttk.Label(ligne, text=details, width=15, anchor='e').pack(side=tk.LEFT, padx=2)
            btn = ttk.Button(ligne, text="Détails", 
                           command=lambda s=stat: afficher_details_rarete(s))
            btn.pack(side=tk.LEFT, padx=2)
        canvas.pack(side=tk.LEFT, fill=tk.BOTH, expand=True)
        scrollbar.pack(side=tk.RIGHT, fill=tk.Y)

    def afficher_details_rarete(stat):
        for widget in panneau_droite.winfo_children():
            widget.destroy()
        if not stat['raretes']:
            ttk.Label(panneau_droite, text="Aucune donnée de rareté disponible").pack(pady=10)
            return
        ttk.Label(panneau_droite, 
                 text=f"Détails - {stat['nom']}", 
                 font=('Helvetica', 10, 'bold')).pack(pady=5)
        raretes_triees = sorted(stat['raretes'].items(), key=lambda x: x[0])
        for rarete, data in raretes_triees:
            cadre_rarete = ttk.LabelFrame(panneau_droite, text=rarete, padding=5)
            cadre_rarete.pack(fill=tk.X, padx=5, pady=2)
            cadre_progression = ttk.Frame(cadre_rarete)
            cadre_progression.pack(fill=tk.X, pady=2)
            progression = ttk.Progressbar(cadre_progression, 
                                     orient=tk.HORIZONTAL, 
                                     length=100, 
                                     mode='determinate')
            progression['value'] = data['pourcentage']
            progression.pack(side=tk.LEFT, fill=tk.X, expand=True)
            details = f"{data['possedees']} / {data['total']} ({data['pourcentage']:.1f}%)"
            ttk.Label(cadre_progression, text=details, width=15).pack(side=tk.LEFT, padx=5)

    btn_frame = ttk.Frame(cadre)
    btn_frame.pack(fill=tk.X, pady=5)
    btn_refresh = ttk.Button(btn_frame, text="Rafraîchir", 
                           command=afficher_stats_globales)
    btn_refresh.pack(side=tk.RIGHT, padx=10)
    afficher_stats_globales()
