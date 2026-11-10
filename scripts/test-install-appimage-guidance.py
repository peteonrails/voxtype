#!/usr/bin/env python3
"""Guard installation guidance until AppImages are actually published.

Run with: python3 scripts/test-install-appimage-guidance.py
This checks documentation only; it never downloads or executes Voxtype.
"""
from pathlib import Path
import re
import unittest


class AppImageGuidanceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.guide = (Path(__file__).resolve().parents[1] / "docs/INSTALL.md").read_text()
        cls.section = cls.guide.split("### AppImage\n", 1)[1].split("\n### ", 1)[0]

    def test_section_states_downloads_are_unavailable(self):
        self.assertIn("AppImage builds are not currently published", self.section)

    def test_section_does_not_offer_a_fabricated_download(self):
        self.assertNotRegex(self.section, r"https?://\S+\.AppImage")
        self.assertNotIn("AppImage variants ship", self.section)

    def test_quick_install_matches_the_section(self):
        summary = re.search(r"^- \*\*AppImage:\*\* (.+)$", self.guide, re.MULTILINE)
        self.assertIsNotNone(summary)
        assert summary is not None
        self.assertIn("not currently published", summary.group(1))
        self.assertIn("[build from source](#build-from-source)", self.section)


if __name__ == "__main__":
    unittest.main()
