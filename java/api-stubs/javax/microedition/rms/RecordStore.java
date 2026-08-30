package javax.microedition.rms;

public class RecordStore {
    private RecordStore() {}

    public static RecordStore openRecordStore(String name, boolean createIfNecessary)
            throws RecordStoreException, RecordStoreFullException, RecordStoreNotFoundException {
        return new RecordStore();
    }

    public static void deleteRecordStore(String name)
            throws RecordStoreException, RecordStoreNotFoundException {}

    public void closeRecordStore() throws RecordStoreNotOpenException, RecordStoreException {}

    public int getNumRecords() throws RecordStoreNotOpenException {
        return 0;
    }

    public int getSizeAvailable() throws RecordStoreNotOpenException {
        return 0;
    }

    public int addRecord(byte[] data, int offset, int numBytes)
            throws RecordStoreNotOpenException, RecordStoreException, RecordStoreFullException {
        return 0;
    }

    public void setRecord(int recordId, byte[] newData, int offset, int numBytes)
            throws RecordStoreNotOpenException, RecordStoreException, RecordStoreFullException {}

    public void deleteRecord(int recordId)
            throws RecordStoreNotOpenException, RecordStoreException {}

    public byte[] getRecord(int recordId)
            throws RecordStoreNotOpenException, RecordStoreException {
        return null;
    }
}
