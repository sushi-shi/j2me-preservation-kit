package javax.microedition.lcdui;

public abstract class Canvas extends Displayable {
    public static final int UP = 1;
    public static final int LEFT = 2;
    public static final int RIGHT = 5;
    public static final int DOWN = 6;
    public static final int FIRE = 8;
    public static final int GAME_A = 9;
    public static final int GAME_B = 10;
    public static final int GAME_C = 11;
    public static final int GAME_D = 12;
    public static final int KEY_NUM0 = 48;
    public static final int KEY_NUM1 = 49;
    public static final int KEY_NUM2 = 50;
    public static final int KEY_NUM3 = 51;
    public static final int KEY_NUM4 = 52;
    public static final int KEY_NUM5 = 53;
    public static final int KEY_NUM6 = 54;
    public static final int KEY_NUM7 = 55;
    public static final int KEY_NUM8 = 56;
    public static final int KEY_NUM9 = 57;
    public static final int KEY_STAR = 42;
    public static final int KEY_POUND = 35;

    protected Canvas() {}

    protected abstract void paint(Graphics graphics);

    protected void keyPressed(int keyCode) {}

    protected void keyReleased(int keyCode) {}

    protected void keyRepeated(int keyCode) {}

    protected void pointerPressed(int x, int y) {}

    protected void pointerDragged(int x, int y) {}

    protected void pointerReleased(int x, int y) {}

    protected void sizeChanged(int width, int height) {}

    protected void hideNotify() {}

    protected void showNotify() {}

    public int getGameAction(int keyCode) {
        return 0;
    }

    public boolean hasPointerEvents() {
        return false;
    }

    public boolean hasPointerMotionEvents() {
        return false;
    }

    public boolean hasRepeatEvents() {
        return false;
    }

    public int getWidth() {
        return 128;
    }

    public int getHeight() {
        return 128;
    }

    public final void repaint() {}

    public final void serviceRepaints() {}

    public void setFullScreenMode(boolean fullScreen) {}
}
