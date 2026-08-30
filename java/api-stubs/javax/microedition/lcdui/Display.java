package javax.microedition.lcdui;

import javax.microedition.midlet.MIDlet;

public class Display {
    private Display() {}

    public static Display getDisplay(MIDlet midlet) {
        return new Display();
    }

    public void setCurrent(Displayable displayable) {}

    public Displayable getCurrent() {
        return null;
    }

    public boolean vibrate(int duration) {
        return false;
    }
}
