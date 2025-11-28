/**
 * Voxtype Demo Animation
 * Simulates Voxtype in action on a Hyprland desktop
 */

class VoxtypeDemo {
    constructor() {
        this.currentScenario = 'agent';
        this.isPlaying = false;
        this.animationTimeout = null;

        // DOM Elements
        this.elements = {
            scenarioButtons: document.querySelectorAll('.scenario-btn'),
            playBtn: document.getElementById('play-btn'),
            resetBtn: document.getElementById('reset-btn'),
            notifications: document.getElementById('notifications'),
            recordingIndicator: document.getElementById('recording-indicator'),
            keyVisual: document.querySelector('.key-visual'),
            keyStatus: document.getElementById('key-status'),
            activeWindowTitle: document.getElementById('active-window-title'),
            scenarioDescription: document.getElementById('scenario-description'),
            // Windows
            windowAgent: document.getElementById('window-agent'),
            windowEditor: document.getElementById('window-editor'),
            windowVim: document.getElementById('window-vim'),
            // Input elements
            agentInput: document.getElementById('agent-input'),
            editorInput: document.getElementById('editor-input'),
            vimInput: document.getElementById('vim-input'),
        };

        // Scenario configurations
        this.scenarios = {
            agent: {
                title: 'Dictate an AI Prompt',
                description: 'Watch as Voxtype captures speech and types a prompt directly into an AI coding assistant. Perfect for hands-free interaction with agentic tools.',
                window: 'window-agent',
                windowTitle: 'foot ~ ai-assistant',
                inputElement: 'agentInput',
                text: 'Help me refactor the authentication module to use JWT tokens instead of session cookies.',
                speechDuration: 3500,
            },
            document: {
                title: 'Writing a Document',
                description: 'Dictate your thoughts directly into a document editor. Great for meeting notes, documentation, or any writing task.',
                window: 'window-editor',
                windowTitle: 'gedit - project-notes.md',
                inputElement: 'editorInput',
                text: 'This project aims to provide a seamless voice-to-text experience for Linux users running Wayland compositors.',
                speechDuration: 4000,
            },
            vim: {
                title: 'Coding in Neovim',
                description: 'Voice-powered coding in Vim. Enter insert mode and dictate code comments, documentation, or even code snippets.',
                window: 'window-vim',
                windowTitle: 'nvim main.rs',
                inputElement: 'vimInput',
                text: 'println!("Please enter your name: ");',
                speechDuration: 2500,
            },
        };

        this.init();
    }

    init() {
        // Scenario button listeners
        this.elements.scenarioButtons.forEach(btn => {
            btn.addEventListener('click', () => {
                if (!this.isPlaying) {
                    this.switchScenario(btn.dataset.scenario);
                }
            });
        });

        // Control button listeners
        this.elements.playBtn.addEventListener('click', () => this.play());
        this.elements.resetBtn.addEventListener('click', () => this.reset());

        // Initialize first scenario
        this.switchScenario('agent');

        // Update clock
        this.updateClock();
        setInterval(() => this.updateClock(), 60000);
    }

    switchScenario(scenario) {
        this.currentScenario = scenario;
        const config = this.scenarios[scenario];

        // Update buttons
        this.elements.scenarioButtons.forEach(btn => {
            btn.classList.toggle('active', btn.dataset.scenario === scenario);
        });

        // Update description
        this.elements.scenarioDescription.innerHTML = `
            <h3>${config.title}</h3>
            <p>${config.description}</p>
        `;

        // Update window title in bar
        this.elements.activeWindowTitle.textContent = config.windowTitle;

        // Show correct window
        document.querySelectorAll('.window').forEach(w => w.classList.remove('active'));
        document.getElementById(config.window).classList.add('active');

        // Reset input
        this.resetInput();
    }

    async play() {
        if (this.isPlaying) return;

        this.isPlaying = true;
        this.elements.playBtn.disabled = true;
        this.elements.resetBtn.disabled = true;

        const config = this.scenarios[this.currentScenario];

        try {
            // Phase 1: Press hotkey
            await this.delay(500);
            this.pressKey();
            this.showNotification('recording', 'Push to Talk Active', 'Recording...');

            // Phase 2: Recording (show waveform animation could be added)
            await this.delay(config.speechDuration);

            // Phase 3: Release hotkey
            this.releaseKey();
            this.hideNotification();
            await this.delay(300);

            // Phase 4: Transcribing
            this.setKeyStatus('Transcribing...', 'transcribing');
            this.showNotification('transcribing', 'Push to Talk Inactive', 'Transcribing...');
            await this.delay(1500);
            this.hideNotification();

            // Phase 5: Type the text
            await this.delay(300);
            this.setKeyStatus('Press to record', '');

            // Show transcription notification
            const preview = config.text.length > 50
                ? config.text.substring(0, 50) + '...'
                : config.text;
            this.showNotification('success', 'Transcribed', preview);

            // Type the text
            await this.typeText(config.text, config.inputElement);

            // Hide final notification after a delay
            await this.delay(2000);
            this.hideNotification();

        } catch (e) {
            console.error('Demo error:', e);
        }

        this.isPlaying = false;
        this.elements.playBtn.disabled = false;
        this.elements.resetBtn.disabled = false;
    }

    reset() {
        // Clear any pending animations
        if (this.animationTimeout) {
            clearTimeout(this.animationTimeout);
        }

        // Reset states
        this.isPlaying = false;
        this.elements.playBtn.disabled = false;
        this.elements.resetBtn.disabled = false;

        // Reset key indicator
        this.elements.keyVisual.classList.remove('pressed');
        this.elements.recordingIndicator.classList.remove('active');
        this.setKeyStatus('Press to record', '');

        // Clear notifications
        this.elements.notifications.innerHTML = '';

        // Reset input
        this.resetInput();
    }

    resetInput() {
        this.elements.agentInput.textContent = '';
        this.elements.editorInput.textContent = '';
        this.elements.vimInput.textContent = '';
    }

    pressKey() {
        this.elements.keyVisual.classList.add('pressed');
        this.elements.recordingIndicator.classList.add('active');
        this.setKeyStatus('Recording...', 'recording');
    }

    releaseKey() {
        this.elements.keyVisual.classList.remove('pressed');
        this.elements.recordingIndicator.classList.remove('active');
    }

    setKeyStatus(text, className) {
        this.elements.keyStatus.textContent = text;
        this.elements.keyStatus.className = className || '';
    }

    showNotification(type, title, body) {
        const notification = document.createElement('div');
        notification.className = 'notification';

        let iconSvg = '';
        if (type === 'recording') {
            iconSvg = `<svg class="notification-icon recording" viewBox="0 0 24 24" fill="currentColor">
                <path d="M12 1a3 3 0 0 0-3 3v8a3 3 0 0 0 6 0V4a3 3 0 0 0-3-3z"/>
                <path d="M19 10v2a7 7 0 0 1-14 0v-2"/>
            </svg>`;
        } else if (type === 'transcribing') {
            iconSvg = `<svg class="notification-icon transcribing" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <circle cx="12" cy="12" r="10"/>
                <polyline points="12 6 12 12 16 14"/>
            </svg>`;
        } else if (type === 'success') {
            iconSvg = `<svg class="notification-icon success" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <path d="M22 11.08V12a10 10 0 1 1-5.93-9.14"/>
                <polyline points="22 4 12 14.01 9 11.01"/>
            </svg>`;
        }

        notification.innerHTML = `
            ${iconSvg}
            <div class="notification-content">
                <div class="notification-title">${title}</div>
                <div class="notification-body">${body}</div>
            </div>
        `;

        // Remove existing notifications
        this.elements.notifications.innerHTML = '';
        this.elements.notifications.appendChild(notification);
    }

    hideNotification() {
        const notification = this.elements.notifications.querySelector('.notification');
        if (notification) {
            notification.classList.add('hiding');
            setTimeout(() => {
                notification.remove();
            }, 300);
        }
    }

    async typeText(text, inputElementKey) {
        const element = this.elements[inputElementKey];
        const chars = text.split('');

        for (let i = 0; i < chars.length; i++) {
            element.textContent += chars[i];
            // Variable typing speed for natural feel
            const delay = Math.random() * 30 + 20;
            await this.delay(delay);
        }
    }

    updateClock() {
        const now = new Date();
        const hours = now.getHours().toString().padStart(2, '0');
        const minutes = now.getMinutes().toString().padStart(2, '0');
        document.getElementById('clock').textContent = `${hours}:${minutes}`;
    }

    delay(ms) {
        return new Promise(resolve => {
            this.animationTimeout = setTimeout(resolve, ms);
        });
    }
}

// Initialize demo when DOM is ready
document.addEventListener('DOMContentLoaded', () => {
    window.voxtypeDemo = new VoxtypeDemo();
});
