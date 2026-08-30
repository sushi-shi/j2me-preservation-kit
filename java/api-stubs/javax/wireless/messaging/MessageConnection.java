package javax.wireless.messaging;

import java.io.IOException;
import javax.microedition.io.Connection;

public interface MessageConnection extends Connection {
    String TEXT_MESSAGE = "text";
    String BINARY_MESSAGE = "binary";
    String MULTIPART_MESSAGE = "multipart";

    Message newMessage(String type);

    Message newMessage(String type, String address);

    void send(Message message) throws IOException;
}
