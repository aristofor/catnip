"""
Example host module demonstrating Catnip module loading.

Usage:
    # In the Catnip script, load with: host = import("03_host_module_example", protocol="py")
"""


def add(a, b):
    """Add two numbers."""
    return a + b


def multiply(a, b):
    """Multiply two numbers."""
    return a * b


def power(base, exponent):
    """Calculate base raised to exponent."""
    return base**exponent


def greet(name, formal=False):
    """Greet someone."""
    if formal:
        return f"Good day, {name}."
    return f"Hey {name}!"


class Counter:
    """A simple counter class."""

    def __init__(self, start=0):
        self.value = start

    def increment(self):
        self.value += 1
        return self.value

    def decrement(self):
        self.value -= 1
        return self.value

    def reset(self):
        self.value = 0
        return self.value


def fibonacci_list(n):
    """Generate list of first n Fibonacci numbers."""
    if n <= 0:
        return []
    if n == 1:
        return [0]

    fib = [0, 1]
    for i in range(2, n):
        fib.append(fib[i - 1] + fib[i - 2])
    return fib


def format_data(data, separator=", "):
    """Format data as a string."""
    if isinstance(data, (list, tuple)):
        return separator.join(str(item) for item in data)
    return str(data)
