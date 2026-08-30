package javax.microedition.lcdui;

public class Graphics {
    public static final int HCENTER = 1;
    public static final int VCENTER = 2;
    public static final int LEFT = 4;
    public static final int RIGHT = 8;
    public static final int TOP = 16;
    public static final int BOTTOM = 32;
    public static final int BASELINE = 64;
    public static final int SOLID = 0;
    public static final int DOTTED = 1;

    public void setColor(int rgb) {}

    public void setColor(int red, int green, int blue) {}

    public int getColor() {
        return 0;
    }

    public void setFont(Font font) {}

    public Font getFont() {
        return Font.getDefaultFont();
    }

    public void setClip(int x, int y, int width, int height) {}

    public void clipRect(int x, int y, int width, int height) {}

    public int getClipX() {
        return 0;
    }

    public int getClipY() {
        return 0;
    }

    public int getClipWidth() {
        return 0;
    }

    public int getClipHeight() {
        return 0;
    }

    public void translate(int x, int y) {}

    public int getTranslateX() {
        return 0;
    }

    public int getTranslateY() {
        return 0;
    }

    public void drawLine(int x1, int y1, int x2, int y2) {}

    public void drawRect(int x, int y, int width, int height) {}

    public void fillRect(int x, int y, int width, int height) {}

    public void drawRoundRect(int x, int y, int width, int height, int arcWidth, int arcHeight) {}

    public void fillRoundRect(int x, int y, int width, int height, int arcWidth, int arcHeight) {}

    public void drawArc(int x, int y, int width, int height, int startAngle, int arcAngle) {}

    public void fillArc(int x, int y, int width, int height, int startAngle, int arcAngle) {}

    public void drawString(String text, int x, int y, int anchor) {}

    public void drawSubstring(String text, int offset, int length, int x, int y, int anchor) {}

    public void drawChar(char character, int x, int y, int anchor) {}

    public void drawChars(char[] data, int offset, int length, int x, int y, int anchor) {}

    public void drawImage(Image image, int x, int y, int anchor) {}
}
