// Voxtype Website JavaScript

document.addEventListener('DOMContentLoaded', () => {
    // Mobile navigation toggle
    const navToggle = document.querySelector('.nav-toggle');
    const navLinks = document.querySelector('.nav-links');

    if (navToggle && navLinks) {
        navToggle.addEventListener('click', () => {
            navLinks.classList.toggle('active');
        });

        // Close menu when clicking a link
        navLinks.querySelectorAll('a').forEach(link => {
            link.addEventListener('click', () => {
                navLinks.classList.remove('active');
            });
        });
    }

    // Installation tabs
    const tabBtns = document.querySelectorAll('.tab-btn');
    const tabContents = document.querySelectorAll('.tab-content');

    tabBtns.forEach(btn => {
        btn.addEventListener('click', () => {
            const target = btn.dataset.tab;

            // Remove active class from all
            tabBtns.forEach(b => b.classList.remove('active'));
            tabContents.forEach(c => c.classList.remove('active'));

            // Add active class to clicked
            btn.classList.add('active');
            document.getElementById(target)?.classList.add('active');
        });
    });

    // Setup tabs (Standard vs Hyprland/Sway)
    const setupTabBtns = document.querySelectorAll('.setup-tab-btn');
    const setupContents = document.querySelectorAll('.setup-content');

    setupTabBtns.forEach(btn => {
        btn.addEventListener('click', () => {
            const target = btn.dataset.setup;

            // Remove active class from all
            setupTabBtns.forEach(b => b.classList.remove('active'));
            setupContents.forEach(c => c.classList.remove('active'));

            // Add active class to clicked
            btn.classList.add('active');
            document.getElementById(`setup-${target}`)?.classList.add('active');
        });
    });

    // Copy button functionality
    const copyBtns = document.querySelectorAll('.copy-btn');

    copyBtns.forEach(btn => {
        btn.addEventListener('click', async () => {
            const codeBlock = btn.closest('.code-block');
            const code = codeBlock?.querySelector('pre code');

            if (code) {
                // Get text content, removing HTML tags
                const text = code.textContent || code.innerText;

                try {
                    await navigator.clipboard.writeText(text);
                    const originalText = btn.textContent;
                    btn.textContent = 'Copied!';
                    btn.style.color = 'var(--color-success)';

                    setTimeout(() => {
                        btn.textContent = originalText;
                        btn.style.color = '';
                    }, 2000);
                } catch (err) {
                    console.error('Failed to copy:', err);
                }
            }
        });
    });

    // Smooth scroll for anchor links
    document.querySelectorAll('a[href^="#"]').forEach(anchor => {
        anchor.addEventListener('click', function (e) {
            e.preventDefault();
            const target = document.querySelector(this.getAttribute('href'));

            if (target) {
                const navHeight = document.querySelector('.navbar')?.offsetHeight || 0;
                const targetPosition = target.getBoundingClientRect().top + window.pageYOffset - navHeight;

                window.scrollTo({
                    top: targetPosition,
                    behavior: 'smooth'
                });
            }
        });
    });

    // Navbar background on scroll
    const navbar = document.querySelector('.navbar');

    if (navbar) {
        window.addEventListener('scroll', () => {
            if (window.scrollY > 50) {
                navbar.style.background = 'rgba(13, 17, 23, 0.98)';
            } else {
                navbar.style.background = 'rgba(13, 17, 23, 0.95)';
            }
        });
    }

    // Intersection Observer for animations
    const observerOptions = {
        root: null,
        rootMargin: '0px',
        threshold: 0.1
    };

    const observer = new IntersectionObserver((entries) => {
        entries.forEach(entry => {
            if (entry.isIntersecting) {
                entry.target.classList.add('animate-in');
            }
        });
    }, observerOptions);

    // Observe feature cards and demo steps
    document.querySelectorAll('.feature-card, .demo-step, .compositor-card').forEach(el => {
        observer.observe(el);
    });
});

// Add animation styles dynamically
const style = document.createElement('style');
style.textContent = `
    .feature-card,
    .demo-step,
    .compositor-card {
        opacity: 0;
        transform: translateY(20px);
        transition: opacity 0.5s ease, transform 0.5s ease;
    }

    .feature-card.animate-in,
    .demo-step.animate-in,
    .compositor-card.animate-in {
        opacity: 1;
        transform: translateY(0);
    }

    .feature-card:nth-child(2) { transition-delay: 0.1s; }
    .feature-card:nth-child(3) { transition-delay: 0.2s; }
    .feature-card:nth-child(4) { transition-delay: 0.3s; }
    .feature-card:nth-child(5) { transition-delay: 0.4s; }
    .feature-card:nth-child(6) { transition-delay: 0.5s; }

    .demo-step:nth-child(2) { transition-delay: 0.1s; }
    .demo-step:nth-child(3) { transition-delay: 0.2s; }
    .demo-step:nth-child(4) { transition-delay: 0.3s; }

    .compositor-card:nth-child(2) { transition-delay: 0.1s; }
    .compositor-card:nth-child(3) { transition-delay: 0.2s; }
    .compositor-card:nth-child(4) { transition-delay: 0.3s; }
    .compositor-card:nth-child(5) { transition-delay: 0.4s; }
`;
document.head.appendChild(style);
