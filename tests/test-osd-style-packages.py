#!/usr/bin/env python3
"""Every shipped recipe also has a selectable canonical renderer package."""
from pathlib import Path
import tomllib
root = Path(__file__).resolve().parents[1]
recipes = sorted((root/'examples/osd-recipes').glob('*.toml'))
for recipe in recipes:
    path = root/'examples/osd-packages'/recipe.stem/'voxtype-osd.toml'
    assert path.is_file(), f'{recipe.stem}: missing selectable style package'
    expected = tomllib.loads(recipe.read_text())['osd']
    actual = tomllib.loads(path.read_text())
    assert actual['name'] == recipe.stem
    for key in ['layout', 'palette', 'frame', 'visual']:
        assert actual[key] == expected[key], (recipe.stem, key)
print('OSD_SELECTABLE_PACKAGES_PASS', len(recipes))
