def generate_cardmarket_url(card_name: str, set_name: str, rarities: list[str]) -> str:
    """
    Génère une URL Cardmarket pour une carte.
    
    Args:
        card_name (str): Nom de la carte
        set_name (str): Nom du set (card_sets_set_name)
        rarities (list[str]): Liste des rarités de la carte dans ce set (non utilisé)
    
    Returns:
        str: URL complète vers la page de la carte sur Cardmarket
    """
    base_url = "https://www.cardmarket.com/fr/YuGiOh/Products/Singles"
    
    # Nettoyage des noms
    set_name = set_name.replace(' ', '-')
    card_name = card_name.replace(' ', '-')
    
    # Retourne toujours une URL simple sans information de rareté
    return f"{base_url}/{set_name}/{card_name}"
