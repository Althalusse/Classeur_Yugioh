
import requests

# Remplacez 'YOUR_API_KEY' par votre clé API réelle
api_key = "eyJhbGciOiJSUzI1NiJ9.eyJpc3MiOiJjYXJkdHJhZGVyLXByb2R1Y3Rpb24iLCJzdWIiOiJhcHA6MTUzNDUiLCJhdWQiOiJhcHA6MTUzNDUiLCJleHAiOjQ5MDMzNTM3NDMsImp0aSI6IjBjMGEwOWIwLWVmY2EtNDk4MS1hZTZhLWM4YzIzMGIwYzhkZSIsImlhdCI6MTc0NzY4MDE0MywibmFtZSI6IkFsdGhhbHVzc2UgQXBwIDIwMjUwNTE5MTgzNjU2In0.mnq8xzy9zq-jr7l_HJKlJ3eD-s7C28Q69j8_tS5WvzpyyzQ6AfmUhKQhR1HyRGuFYvZog2vw9hnXqBH0juwTY0o-poZ_DSV5URemeEPmYQFwqG3hsgBSADWh0bI3-M78d0bia-5EaguIBfzK7KStMY8cwoOd17PEMtgC-1TKvjEf-KFat5RyIC-SkrH0Thk9fJKvljmv6EOhceULEUfyHvp2x7iuySH759gagrDHrYxuR1twLA7aBiYLueKmVtrqzyPJdhMUfy1j1_SCYh3xSCz17QElDDjfigb-F1EloLV8qsyIm0EZ0LWNOTmvRHpIOZxwHBJqf15tlxqwWKO59g"

card_reference = 'MP24-EN001'
game_id = '4'  # Remplacez par l'identifiant du jeu

# URL de l'API pour rechercher des cartes
url = f'https://api.cardtrader.com/v1/cards?reference={card_reference}&gameId={game_id}'

# En-têtes de la requête
headers = {
    'Authorization': f'Bearer {api_key}',
    'Accept': 'application/json'
}

# Effectuer la requête GET
response = requests.get(url, headers=headers)

# Vérifier si la requête a réussi
if response.status_code == 200:
    data = response.json()
    # Analyser les données pour extraire le prix
    print(data)
else:
    print(f"Erreur: {response.status_code}")
