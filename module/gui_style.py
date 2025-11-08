import tkinter as tk
from tkinter import ttk

class ApplicationStyle:
    # Couleurs principales
    COLORS = {
        'primary': '#2196F3',
        'secondary': '#757575',
        'background': '#f5f5f5',
        'surface': '#ffffff',
        'error': '#B00020',
        'success': '#4CAF50',
        'warning': '#FFC107'
    }

    @staticmethod
    def apply_theme(root):
        style = ttk.Style()
        style.configure('.',
            background=ApplicationStyle.COLORS['background'],
            foreground=ApplicationStyle.COLORS['secondary'])
            
        style.configure('Title.TLabel',
            font=('Helvetica', 16, 'bold'),
            foreground=ApplicationStyle.COLORS['primary'])
            
        style.configure('Card.TFrame',
            background=ApplicationStyle.COLORS['surface'],
            relief='solid',
            borderwidth=1)
