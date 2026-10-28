#!/usr/bin/env python3
"""Regenerate committed application icons from resources/icon.png (requires Pillow)."""
from pathlib import Path

from PIL import Image

resources = Path(__file__).resolve().parents[1] / 'resources'
with Image.open(resources / 'icon.png') as source:
    if source.width != source.height:
        raise ValueError('The application icon must be square.')
    image = source.convert('RGBA')
    image.save(resources / 'icon.ico', sizes=[(n, n) for n in (16, 24, 32, 48, 64, 128, 256)])
    (resources / 'icon.rgba').write_bytes(image.resize((256, 256), Image.Resampling.LANCZOS).tobytes())
print('Updated resources/icon.ico and resources/icon.rgba')
