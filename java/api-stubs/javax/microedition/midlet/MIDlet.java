package javax.microedition.midlet;

public abstract class MIDlet {
    protected MIDlet() {}

    public abstract void startApp() throws MIDletStateChangeException;

    public abstract void pauseApp();

    public abstract void destroyApp(boolean unconditional) throws MIDletStateChangeException;

    public final String getAppProperty(String key) {
        return null;
    }

    public final void notifyDestroyed() {}

    public final boolean platformRequest(String url)
            throws javax.microedition.io.ConnectionNotFoundException {
        return false;
    }

    public final void notifyPaused() {}

    public final void resumeRequest() {}
}

