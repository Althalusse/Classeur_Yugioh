import tkinter as tk
from tkinter import ttk

class YugiohTheme:
    COLORS = {
        'primary': '#2196F3',
        'secondary': '#757575',
        'background': '#f5f5f5',
        'surface': '#ffffff',
        'error': '#B00020',
        'on_primary': '#ffffff',
        'on_secondary': '#ffffff',
        'on_background': '#000000',
        'on_surface': '#000000',
        'on_error': '#ffffff',
    }

    @staticmethod
    def setup():
        style = ttk.Style()
        
        # Configuration générale
        style.configure('.',
            background=YugiohTheme.COLORS['background'],
            foreground=YugiohTheme.COLORS['on_background'],
            font=('Segoe UI', 10))
            
        # Treeview personnalisé
        style.configure('Yugioh.Treeview',
            background=YugiohTheme.COLORS['surface'],
            foreground=YugiohTheme.COLORS['on_surface'],
            rowheight=30,
            fieldbackground=YugiohTheme.COLORS['surface'])
            
        style.configure('Yugioh.Treeview.Heading',
            background=YugiohTheme.COLORS['primary'],
            foreground=YugiohTheme.COLORS['on_primary'],
            font=('Segoe UI', 10, 'bold'))
            
        # Label de titre
        style.configure('Title.TLabel',
            font=('Segoe UI', 12, 'bold'),
            foreground=YugiohTheme.COLORS['primary'])
            
        # Boutons
        style.configure('Primary.TButton',
            background=YugiohTheme.COLORS['primary'],
            foreground=YugiohTheme.COLORS['on_surface'])
            
        # Ajout du style pour le bouton Annuler
        style.configure('Secondary.TButton',
            background=YugiohTheme.COLORS['secondary'],
            foreground=YugiohTheme.COLORS['on_surface'])
