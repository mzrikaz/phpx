
// ---- DOM (natives in crates/interp/src/dom/) ---------------------------------------------

class DOMException extends Exception
{
    public $code = 0;
}

final class LibXMLError
{
    public int $level;
    public int $code;
    public int $column;
    public string $message;
    public string $file;
    public int $line;
}

interface DOMParentNode
{
}

interface DOMChildNode
{
}

class DOMImplementation
{
}

class DOMNode
{
    public const DOCUMENT_POSITION_DISCONNECTED = 1;
    public const DOCUMENT_POSITION_PRECEDING = 2;
    public const DOCUMENT_POSITION_FOLLOWING = 4;
    public const DOCUMENT_POSITION_CONTAINS = 8;
    public const DOCUMENT_POSITION_CONTAINED_BY = 16;
    public const DOCUMENT_POSITION_IMPLEMENTATION_SPECIFIC = 32;
}

class DOMNameSpaceNode
{
}

class DOMDocumentFragment extends DOMNode implements DOMParentNode
{
}

class DOMDocument extends DOMNode implements DOMParentNode
{
}

class DOMNodeList implements IteratorAggregate, Countable
{
}

class DOMNamedNodeMap implements IteratorAggregate, Countable
{
}

class DOMCharacterData extends DOMNode implements DOMChildNode
{
}

class DOMAttr extends DOMNode
{
}

class DOMElement extends DOMNode implements DOMParentNode, DOMChildNode
{
}

class DOMText extends DOMCharacterData
{
}

class DOMComment extends DOMCharacterData
{
}

class DOMCdataSection extends DOMText
{
}

class DOMDocumentType extends DOMNode
{
}

class DOMNotation extends DOMNode
{
}

class DOMEntity extends DOMNode
{
}

class DOMEntityReference extends DOMNode
{
}

class DOMProcessingInstruction extends DOMNode
{
}

class DOMXPath
{
}
