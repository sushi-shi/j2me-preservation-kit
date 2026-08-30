package javax.microedition.lcdui;

import java.io.IOException;
import java.io.InputStream;

public class Image {
    protected Image() {}

    public static Image createImage(int width, int height) {
        return new Image();
    }

    public static Image createImage(String name) throws IOException {
        return new Image();
    }

    public static Image createImage(InputStream stream) throws IOException {
        return new Image();
    }

    public static Image createImage(byte[] imageData, int imageOffset, int imageLength) {
        return new Image();
    }

    public static Image createImage(Image source) {
        return new Image();
    }

    public static Image createRGBImage(int[] rgb, int width, int height, boolean processAlpha) {
        return new Image();
    }

    public Graphics getGraphics() {
        return new Graphics();
    }

    public void getRGB(int[] rgbData, int offset, int scanlength, int x, int y, int width, int height) {}

    public int getWidth() {
        return 0;
    }

    public int getHeight() {
        return 0;
    }
}
