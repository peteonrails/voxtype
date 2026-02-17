import { defineConfig } from "vitepress";

export default defineConfig({
  title: "Voxtype",
  description:
    "Voice-to-text with push-to-talk for Wayland compositors.",
  base: "/docs/",
  srcDir: ".",
  outDir: "../website/docs",

  head: [["link", { rel: "icon", href: "/docs/logo.svg" }]],

  cleanUrls: true,
  lastUpdated: true,
  ignoreDeadLinks: true,
  rewrites: {
    "INSTALL.md": "index.md",
  },
  themeConfig: {
    logo: { light: "/logo.svg", dark: "/logo.svg" },

    outline: { level: "deep" },

    socialLinks: [{ icon: "github", link: "https://github.com/peteonrails/voxtype" }],

    search: { provider: "local" },

    editLink: {
      pattern: "https://github.com/peteonrails/voxtype/edit/master/docs/:path",
    },

    footer: {
      message: "Released under the MIT License",
    },

    

    nav: [
      { text:"Homepage", link:"https://voxtype.io" },
      { text: "Installation", link: "/" },  
      { text: "Troubleshooting", link: "/TROUBLESHOOTING" },
      { text: "FAQ", link: "/FAQ" },
    ],

    sidebar: {
      "/": [
        {
          text: "Getting Started",
          items: [
            { text: "Installation", link: "/" },
            { text: "Why Voxtype?", link: "/why-voxtype" },
          ],
        },
        {
          text: "Core Docs",
          items: [
            { text: "User Manual", link: "/USER_MANUAL" },
            { text: "Configuration", link: "/CONFIGURATION" },
          ],
        },
        {
          text: "Guides",
          items: [
            { text: "Model Selection", link: "/MODEL_SELECTION_GUIDE" },
            { text: "Waybar Integration", link: "/WAYBAR" },
            { text: "Parakeet ASR", link: "/PARAKEET" },
            { text: "CI Setup", link: "/CI-SETUP" },
          ],
        },
        {
          text: "Reference",
          items: [
            { text: "Troubleshooting", link: "/TROUBLESHOOTING" },
            { text: "FAQ", link: "/FAQ" },
            { text: "Smoke Tests", link: "/SMOKE_TESTS" },
          ],
        },
      ],
    },
  },
});
