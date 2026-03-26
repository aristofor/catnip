"""
Example: Application API exposed to Catnip scripts.

This demonstrates how an application can expose its functionality to Catnip
as a scripting language.

Usage:
    # In the Catnip script, load with: app = import("07_app_api", protocol="py")
"""

# Simulated application state
_app_state = {
    'users': [
        {'id': 1, 'name': 'Alice', 'score': 100},
        {'id': 2, 'name': 'Bob', 'score': 85},
        {'id': 3, 'name': 'Charlie', 'score': 92},
    ],
    'config': {'max_score': 100, 'min_score': 0},
}


def get_users():
    """Get all users from the application."""
    return _app_state['users'].copy()


def get_user(user_id):
    """Get a specific user by ID."""
    for user in _app_state['users']:
        if user['id'] == user_id:
            return user.copy()
    return None


def update_score(user_id, new_score):
    """Update a user's score."""
    for user in _app_state['users']:
        if user['id'] == user_id:
            user['score'] = new_score
            return True
    return False


def get_top_scores(limit=3):
    """Get top N users by score."""
    sorted_users = sorted(_app_state['users'], key=lambda u: u['score'], reverse=True)
    return sorted_users[:limit]


def calculate_average_score():
    """Calculate the average score across all users."""
    if not _app_state['users']:
        return 0
    total = sum(user['score'] for user in _app_state['users'])
    return total / len(_app_state['users'])


def get_config(key):
    """Get a configuration value."""
    return _app_state['config'].get(key)


def log_message(message):
    """Log a message (simulated)."""
    print(f"[APP LOG] {message}")
    return True


class Report:
    """Generate reports from application data."""

    def __init__(self):
        self.lines = []

    def add_line(self, text):
        """Add a line to the report."""
        self.lines.append(str(text))

    def add_separator(self):
        """Add a separator line."""
        self.lines.append("-" * 40)

    def generate(self):
        """Generate the final report as a string."""
        return "\n".join(self.lines)

    def clear(self):
        """Clear the report."""
        self.lines = []
