#!/bin/bash
# Test script for voxtype package installation
# Runs inside Docker containers to verify packages work correctly
#
# Usage: ./test-install.sh [debian|rpm|arch] [package-path]

set -e

DISTRO_TYPE="${1:-debian}"
PACKAGE_PATH="${2:-}"
PASS=0
FAIL=0

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log_pass() {
    echo -e "${GREEN}[PASS]${NC} $1"
    ((PASS++)) || true
}

log_fail() {
    echo -e "${RED}[FAIL]${NC} $1"
    ((FAIL++)) || true
}

log_info() {
    echo -e "${YELLOW}[INFO]${NC} $1"
}

# Detect CPU capabilities
detect_cpu() {
    log_info "CPU: $(grep 'model name' /proc/cpuinfo | head -1 | cut -d: -f2 | xargs)"

    if grep -q avx512f /proc/cpuinfo; then
        log_info "CPU supports: AVX-512"
        CPU_LEVEL="avx512"
    elif grep -q avx2 /proc/cpuinfo; then
        log_info "CPU supports: AVX2 (no AVX-512)"
        CPU_LEVEL="avx2"
    elif grep -q avx /proc/cpuinfo; then
        log_info "CPU supports: AVX (no AVX2)"
        CPU_LEVEL="avx"
    else
        log_info "CPU supports: SSE only (no AVX)"
        CPU_LEVEL="sse"
    fi
}

# Find package file
find_package() {
    local pkg_type="$1"
    local pkg_dir="/packages"

    if [[ -n "$PACKAGE_PATH" && -f "$PACKAGE_PATH" ]]; then
        echo "$PACKAGE_PATH"
        return 0
    fi

    # Find latest version directory
    local latest_dir
    latest_dir=$(ls -d ${pkg_dir}/*/ 2>/dev/null | sort -V | tail -1)

    if [[ -z "$latest_dir" ]]; then
        # Try flat structure
        latest_dir="$pkg_dir"
    fi

    case "$pkg_type" in
        debian)
            find "$latest_dir" -name "*.deb" 2>/dev/null | head -1
            ;;
        rpm)
            find "$latest_dir" -name "*.rpm" 2>/dev/null | head -1
            ;;
        arch)
            find "$latest_dir" -name "*.pkg.tar.*" 2>/dev/null | head -1
            ;;
    esac
}

# Install package based on distro type
install_package() {
    local pkg_type="$1"
    local pkg_file="$2"

    log_info "Installing: $pkg_file"

    case "$pkg_type" in
        debian)
            dpkg -i "$pkg_file" 2>&1 || apt-get install -f -y 2>&1
            ;;
        rpm)
            rpm -i "$pkg_file" 2>&1 || dnf install -y "$pkg_file" 2>&1
            ;;
        arch)
            pacman -U --noconfirm "$pkg_file" 2>&1
            ;;
    esac
}

# Test 1: Package installs without errors
test_installation() {
    local pkg_type="$1"
    local pkg_file="$2"

    if install_package "$pkg_type" "$pkg_file"; then
        log_pass "Package installed successfully"
        return 0
    else
        log_fail "Package installation failed"
        return 1
    fi
}

# Test 2: Binary exists and is executable
test_binary_exists() {
    if [[ -x /usr/bin/voxtype ]]; then
        log_pass "Binary exists at /usr/bin/voxtype"

        # Check if it's a symlink (tiered binary setup)
        if [[ -L /usr/bin/voxtype ]]; then
            local target
            target=$(readlink -f /usr/bin/voxtype)
            log_info "Binary is symlink to: $target"
        fi
        return 0
    else
        log_fail "Binary not found or not executable at /usr/bin/voxtype"
        return 1
    fi
}

test_gguf_worker_exists() {
    if [[ -x /usr/lib/voxtype/voxtype-gguf-worker || -x /usr/bin/voxtype-gguf-worker ]]; then
        log_pass "GGUF worker is installed beside voxtype"
    else
        log_fail "GGUF worker is missing from the package"
        return 1
    fi

    local directory
    for directory in /usr/lib/voxtype /usr/bin; do
        if [[ -x "$directory/voxtype-vulkan" && ! -x "$directory/voxtype-gguf-worker-vulkan" ]]; then
            log_fail "Vulkan GGUF worker is missing or not executable beside $directory/voxtype-vulkan"
            return 1
        fi
    done
}

# Test 3: Binary runs without SIGILL
test_binary_runs() {
    log_info "Testing binary execution (checking for SIGILL)..."

    # Run with timeout to prevent hangs, capture exit code
    set +e
    timeout 5 /usr/bin/voxtype --version > /tmp/version_output.txt 2>&1
    local exit_code=$?
    set -e

    # Check for SIGILL (exit code 132 = 128 + 4)
    if [[ $exit_code -eq 132 ]]; then
        log_fail "Binary crashed with SIGILL (illegal instruction)"
        log_info "This indicates CPU instruction mismatch (AVX-512/AVX2)"
        return 1
    elif [[ $exit_code -eq 0 ]]; then
        local version
        version=$(cat /tmp/version_output.txt)
        log_pass "Binary runs successfully: $version"
        return 0
    elif [[ $exit_code -eq 124 ]]; then
        log_fail "Binary timed out (may be hanging)"
        return 1
    else
        # Other exit codes might be OK (e.g., missing config)
        log_info "Binary exited with code $exit_code (not SIGILL)"
        cat /tmp/version_output.txt
        log_pass "Binary runs without SIGILL crash"
        return 0
    fi
}

# Test 4: Help command works
test_help_command() {
    set +e
    timeout 5 /usr/bin/voxtype --help > /tmp/help_output.txt 2>&1
    local exit_code=$?
    set -e

    if [[ $exit_code -eq 0 ]] && grep -q "voxtype" /tmp/help_output.txt; then
        log_pass "Help command works"
        return 0
    elif [[ $exit_code -eq 132 ]]; then
        log_fail "Help command crashed with SIGILL"
        return 1
    else
        log_info "Help command exited with code $exit_code"
        log_pass "Help command runs without crash"
        return 0
    fi
}

# Test 5: Verify correct binary variant is selected (for tiered builds)
test_binary_variant() {
    if [[ -L /usr/bin/voxtype ]]; then
        local target
        target=$(readlink -f /usr/bin/voxtype)

        if [[ "$CPU_LEVEL" == "avx512" ]] && [[ "$target" == *"avx512"* ]]; then
            log_pass "Correct binary variant selected (AVX-512)"
        elif [[ "$CPU_LEVEL" != "avx512" ]] && [[ "$target" == *"avx2"* ]]; then
            log_pass "Correct binary variant selected (AVX2)"
        elif [[ "$target" == *"avx512"* ]] && [[ "$CPU_LEVEL" != "avx512" ]]; then
            log_fail "Wrong binary variant: AVX-512 selected but CPU only supports $CPU_LEVEL"
        else
            log_info "Binary variant: $target (CPU: $CPU_LEVEL)"
            log_pass "Binary variant check completed"
        fi
    else
        log_info "Single binary (not tiered) - skipping variant check"
    fi
}

# Main test sequence
main() {
    echo "========================================"
    echo "Voxtype Package Test Suite"
    echo "Distro type: $DISTRO_TYPE"
    echo "========================================"
    echo ""

    # Detect CPU
    detect_cpu
    echo ""

    # Find package
    local pkg_file
    pkg_file=$(find_package "$DISTRO_TYPE")

    if [[ -z "$pkg_file" || ! -f "$pkg_file" ]]; then
        log_fail "No package file found for $DISTRO_TYPE"
        echo ""
        echo "Expected packages in /packages directory"
        echo "Contents of /packages:"
        ls -la /packages/ 2>/dev/null || echo "  (directory not found)"
        exit 1
    fi

    log_info "Found package: $pkg_file"
    echo ""

    # Run tests
    echo "--- Installation Test ---"
    test_installation "$DISTRO_TYPE" "$pkg_file"
    echo ""

    echo "--- Binary Tests ---"
    test_binary_exists
    test_gguf_worker_exists
    test_binary_runs
    test_help_command
    test_binary_variant
    echo ""

    # Summary
    echo "========================================"
    echo "Test Summary"
    echo "========================================"
    echo -e "Passed: ${GREEN}$PASS${NC}"
    echo -e "Failed: ${RED}$FAIL${NC}"
    echo ""

    if [[ $FAIL -gt 0 ]]; then
        echo -e "${RED}TESTS FAILED${NC}"
        exit 1
    else
        echo -e "${GREEN}ALL TESTS PASSED${NC}"
        exit 0
    fi
}

main "$@"
