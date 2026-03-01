"""Thin AES-GCM wrapper — logic stays in Catnip."""

from cryptography.hazmat.primitives.ciphers.aead import AESGCM


def make_key(size=32):
    return AESGCM.generate_key(bit_length=size * 8)


def encrypt(key, nonce, data, aad=None):
    return AESGCM(key).encrypt(nonce, data, aad)


def decrypt(key, nonce, ct, aad=None):
    return AESGCM(key).decrypt(nonce, ct, aad)
