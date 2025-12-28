#!/bin/bash
set -e

# Configuration
IMAGE_NAME="postrust/postrust"
VERSION=$(grep -m1 'version = ' Cargo.toml | sed 's/.*"\(.*\)".*/\1/')

echo "=== Postrust Docker Build & Push ==="
echo "Version: $VERSION"
echo ""

# Parse arguments
PUSH=false
LATEST=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --push)
            PUSH=true
            shift
            ;;
        --latest)
            LATEST=true
            shift
            ;;
        --version)
            VERSION="$2"
            shift 2
            ;;
        --image)
            IMAGE_NAME="$2"
            shift 2
            ;;
        -h|--help)
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  --push        Push image to Docker Hub after building"
            echo "  --latest      Also tag and push as 'latest'"
            echo "  --version     Override version (default: from Cargo.toml)"
            echo "  --image       Override image name (default: postrust/postrust)"
            echo "  -h, --help    Show this help message"
            echo ""
            echo "Examples:"
            echo "  $0                          # Build only"
            echo "  $0 --push                   # Build and push"
            echo "  $0 --push --latest          # Build and push with latest tag"
            echo "  $0 --image myuser/postrust  # Use custom image name"
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

echo "Image: $IMAGE_NAME:$VERSION"
echo ""

# Build the Docker image
echo "=== Step 1: Building Docker image ==="
docker build -t "$IMAGE_NAME:$VERSION" .

if [ "$LATEST" = true ]; then
    echo ""
    echo "=== Tagging as latest ==="
    docker tag "$IMAGE_NAME:$VERSION" "$IMAGE_NAME:latest"
fi

echo ""
echo "Build complete!"
echo "  - $IMAGE_NAME:$VERSION"
if [ "$LATEST" = true ]; then
    echo "  - $IMAGE_NAME:latest"
fi

# Push to Docker Hub
if [ "$PUSH" = true ]; then
    echo ""
    echo "=== Step 2: Pushing to Docker Hub ==="

    # Check if logged in
    if ! docker info 2>/dev/null | grep -q "Username"; then
        echo "Not logged in to Docker Hub. Please run 'docker login' first."
        exit 1
    fi

    echo "Pushing $IMAGE_NAME:$VERSION..."
    docker push "$IMAGE_NAME:$VERSION"

    if [ "$LATEST" = true ]; then
        echo "Pushing $IMAGE_NAME:latest..."
        docker push "$IMAGE_NAME:latest"
    fi

    echo ""
    echo "Push complete!"
    echo ""
    echo "Image available at:"
    echo "  docker pull $IMAGE_NAME:$VERSION"
    if [ "$LATEST" = true ]; then
        echo "  docker pull $IMAGE_NAME:latest"
    fi
else
    echo ""
    echo "To push to Docker Hub, run:"
    echo "  $0 --push"
    echo ""
    echo "Or manually:"
    echo "  docker push $IMAGE_NAME:$VERSION"
fi

echo ""
echo "Done!"
