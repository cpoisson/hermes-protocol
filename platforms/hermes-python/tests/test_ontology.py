from __future__ import unicode_literals
from collections import defaultdict
import pytest

import hermes_python


def test_slot_access():
    custom_slot_value = hermes_python.ontology.dialogue.CustomValue("custom_slot")
    slot_value = hermes_python.ontology.dialogue.SlotValue(1, custom_slot_value)
    nlu_slot = hermes_python.ontology.dialogue.NluSlot(0.8, slot_value, custom_slot_value.value, "entity", "test_slot", 0, 100)

    slot_map = defaultdict(hermes_python.ontology.dialogue.SlotsList)
    slot_map[nlu_slot.slot_name].append(nlu_slot)

    slots = hermes_python.ontology.dialogue.SlotMap(slot_map)
    intent_message = hermes_python.ontology.dialogue.IntentMessage("session_id", "", "site_id", "input", "testIntent", slots)
    assert type(intent_message.slots.test_slot.first()) is hermes_python.ontology.dialogue.CustomValue
    assert type(intent_message.slots.test_slot.all()[0]) is hermes_python.ontology.dialogue.CustomValue
    assert type(intent_message.slots.test_slot[0]) is hermes_python.ontology.dialogue.NluSlot
    assert type(intent_message.slots.test_slot[0].slot_value) is hermes_python.ontology.dialogue.SlotValue
    assert type(intent_message.slots.test_slot[0].slot_value.value) is hermes_python.ontology.dialogue.CustomValue


def test_unexisting_slot_access():
    ""

def test_confidence_access():
    custom_slot_value = hermes_python.ontology.dialogue.CustomValue("custom_slot")
    slot_value = hermes_python.ontology.dialogue.SlotValue(1, custom_slot_value)
    nlu_slot = hermes_python.ontology.dialogue.NluSlot(0.8, slot_value, custom_slot_value.value, "entity", "test_slot", 0, 100)

    slot_map = defaultdict(hermes_python.ontology.dialogue.SlotsList)
    slot_map[nlu_slot.slot_name].append(nlu_slot)

    slots = hermes_python.ontology.dialogue.SlotMap(slot_map)
    intent_message = hermes_python.ontology.dialogue.IntentMessage("session_id", "", "site_id", "input", "testIntent", slots)
    assert intent_message.slots.test_slot[0].confidence_score == 0.8


