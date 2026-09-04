import QtQuick;

/**
 * Represents a message in an AI conversation. (Kind of) follows the OpenAI API message structure.
 */
QtObject {
    property string role
    property string content
    property string rawContent
    // First-class stream blocks.  The Souveraine adapter fills these from the
    // typed SSE events; legacy/provider messages leave this empty and the
    // existing markdown splitter remains their renderer.
    property var segments: []
    property string fileMimeType
    property string fileUri
    property string localFilePath
    property string model
    property bool thinking: true
    property bool done: false
    property var annotations: []
    property var annotationSources: []
    property list<string> searchQueries: []
    property string functionName
    property var functionCall
    property string thoughtSignature
    property string functionResponse
    property bool functionPending: false
    property bool visibleToUser: true
}
