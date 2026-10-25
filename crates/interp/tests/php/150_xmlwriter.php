<?php
$w = new XMLWriter; $w->openMemory(); $w->setIndent(true);
$w->startDocument('1.0', 'UTF-8');
$w->startElement('root');
$w->writeAttribute('a', 'x"é<');
$w->writeElement('a', '1 & 2');
$w->startElement('b'); $w->writeAttribute('x', 'y'); $w->text('t'); $w->endElement();
$w->writeElement('empty');
$w->writeElement('emptystr', '');
$w->startElement('c'); $w->text('mixed'); $w->writeElement('d', 'in'); $w->endElement();
$w->writeComment(' note ');
$w->startElementNs('p', 'ns', 'urn:p'); $w->writeAttributeNs('p', 'k', 'urn:p', 'v'); $w->endElement();
$w->writeCdata('raw <x>');
$w->writePi('php', 'echo 1;');
$w->startElement('full'); $w->fullEndElement();
$w->endElement();
$w->endDocument();
echo $w->outputMemory();
echo "---\n";
$w = xmlwriter_open_memory(); xmlwriter_start_element($w, 'r'); xmlwriter_text($w, 'é'); xmlwriter_start_element($w, 's'); xmlwriter_end_element($w); xmlwriter_end_element($w); echo xmlwriter_output_memory($w), "\n";
$w = new XMLWriter; $w->openMemory(); $w->setIndent(true); $w->setIndentString("\t"); $w->startElement('a'); $w->startElement('b'); $w->startElement('c'); $w->text('x'); $w->endElement(); $w->endElement(); $w->endElement(); echo $w->outputMemory();
$w = new XMLWriter; $w->openMemory(); $w->startDocument(); $w->startElement('x'); $w->endDocument(); var_dump($w->outputMemory(false), $w->outputMemory());
try { $w->startElement('1bad'); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
