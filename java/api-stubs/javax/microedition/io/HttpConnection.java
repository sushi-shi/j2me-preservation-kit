package javax.microedition.io;

import java.io.IOException;

public interface HttpConnection extends InputConnection, OutputConnection {
    String GET = "GET";
    String POST = "POST";
    String HEAD = "HEAD";

    void setRequestMethod(String method) throws IOException;

    void setRequestProperty(String key, String value) throws IOException;

    int getResponseCode() throws IOException;
}
