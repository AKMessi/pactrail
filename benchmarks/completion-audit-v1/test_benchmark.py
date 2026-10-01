"""Offline grader validation; no model requests and no loopback service."""
import pathlib
import tempfile
import unittest
from cases import CASES
from run import ARMS, changed_paths, grade, setup, write
from summarize import summarize

class GraderValidation(unittest.TestCase):
    def test_every_broken_baseline_fails_and_gold_passes(self):
        with tempfile.TemporaryDirectory(prefix='pactrail-grader-tests-') as tmp:
            for case in CASES:
                with self.subTest(case=case['id']):
                    broken=pathlib.Path(tmp)/(case['id']+'-broken')
                    fixed=pathlib.Path(tmp)/(case['id']+'-fixed')
                    setup(broken,case);setup(fixed,case,True)
                    self.assertFalse(grade(broken,case)['passed'])
                    self.assertTrue(grade(fixed,case)['passed'])

    def test_import_exit_cannot_masquerade_as_completed_grader(self):
        with tempfile.TemporaryDirectory(prefix='pactrail-grader-tests-') as tmp:
            workspace=pathlib.Path(tmp)/'workspace'
            case={'files':{'bad.py':'import sys\nsys.exit(0)\n'},'grader':'import bad\nassert False\n'}
            setup(workspace,case)
            self.assertFalse(grade(workspace,case)['passed'])

class ResultAccounting(unittest.TestCase):
    def test_empty_unrelated_files_are_still_changes(self):
        original={'production.py':'pass\n','deleted_empty.py':''}
        actual={'production.py':b'pass\n','added_empty.py':b''}
        self.assertEqual(changed_paths(original,actual),['added_empty.py','deleted_empty.py'])
        self.assertEqual(changed_paths({'same.py':''},{'same.py':b''}),[])

    def test_pending_and_ungraded_trials_do_not_become_zero_scores(self):
        with tempfile.TemporaryDirectory(prefix='pactrail-result-tests-') as tmp:
            folder=pathlib.Path(tmp)
            write(folder/'protocol.json',{'order':[{'arm':arm} for arm in ARMS]})
            pending=summarize(folder)
            self.assertEqual(pending['pending_trials'],4)
            self.assertIsNone(pending['arms']['pactrail-audit']['functional_passes'])
            write(folder/'results.json',[{'case':'test','repetition':1,'arm':'pactrail-audit',
                  'functional':None,'strict_passed':False,'requests':1,'usage':[None]}])
            result=summarize(folder)
            self.assertEqual(result['pending_trials'],3)
            self.assertIsNone(result['arms']['pactrail-audit']['functional_passes'])
            self.assertIsNone(result['arms']['pactrail-audit']['token_usage']['total_tokens']['reported_sum'])
            self.assertEqual(result['paired_functional']['pactrail-baseline']['pairs'],0)

if __name__=='__main__': unittest.main()
